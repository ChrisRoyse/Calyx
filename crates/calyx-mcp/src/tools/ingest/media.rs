use std::collections::BTreeMap;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

use calyx_core::{CalyxError, Input, Modality, SlotState, VaultStore, media_modality_name};
use calyx_ledger::{ActorId, SubjectId};
use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::protocol::ToolDef;
use crate::schema::{object_schema, string_schema};
use crate::server::{McpServer, Tool};
use crate::server::{ToolError, ToolResult};
use crate::tools::vault::now_ms;
use crate::tools::vault::store::ResolvedVault;

use super::input_retention::{INPUT_POINTER_PREFIX, write_input_blob};
use super::{
    base_exists, decode, def, derived_text, enum_string, measure_constellation, open_vault,
    resolve_requested_vault,
};

pub(super) fn register(server: &mut McpServer) -> Result<(), CalyxError> {
    server.register(Box::new(MediaIngestTool))
}

struct MediaIngestTool;

#[derive(Debug)]
pub(super) struct RetainedMediaInput {
    pub(super) input: Input,
    pub(super) metadata: BTreeMap<String, String>,
    pub(super) pointer: String,
    pub(super) source_sha256: String,
    pub(super) input_blake3: [u8; 32],
}

#[derive(Deserialize)]
struct MediaIngestArgs {
    vault: String,
    file: PathBuf,
    modality: String,
}

#[derive(Debug)]
struct MediaProbe {
    codec: String,
    container: String,
    duration_seconds: Option<f64>,
    sample_rate_hz: Option<u32>,
    channels: Option<u32>,
    width: Option<u32>,
    height: Option<u32>,
    frame_count: Option<u64>,
    fps: Option<f64>,
}

pub(super) fn parse_audio_video_modality(raw: &str) -> ToolResult<Modality> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "image" => Ok(Modality::Image),
        "audio" => Ok(Modality::Audio),
        "video" => Ok(Modality::Video),
        other => Err(ToolError::invalid_params(format!(
            "unsupported raw media modality {other}; expected image, audio, or video"
        ))),
    }
}

impl Tool for MediaIngestTool {
    fn def(&self) -> ToolDef {
        def(
            "calyx.ingest_media",
            "ingest retained image/audio/video bytes into a Calyx vault",
            "store raw media bytes -> derived text -> linked constellations",
            object_schema(&[
                ("vault", string_schema(), true),
                ("file", string_schema(), true),
                ("modality", enum_string(&["image", "audio", "video"]), true),
            ]),
        )
    }

    fn call(&self, params: Value) -> ToolResult<Value> {
        let args: MediaIngestArgs = decode("calyx.ingest_media", params)?;
        let modality = parse_audio_video_modality(&args.modality)?;
        let resolved = resolve_requested_vault(&args.vault)?;
        let retained = retain_media_input(&resolved, args.file.as_ref(), modality)?;
        let reports = derived::ingest_media_with_derived_text(&resolved, retained)?;
        Ok(
            serde_json::to_value(serde_json::json!({ "results": reports })).map_err(|err| {
                CalyxError::aster_corrupt_shard(format!("encode media ingest: {err}"))
            })?,
        )
    }

    fn requires_authn(&self) -> bool {
        true
    }
}

pub(super) fn retain_media_input(
    resolved: &ResolvedVault,
    source: &Path,
    modality: Modality,
) -> ToolResult<RetainedMediaInput> {
    let extension = media_extension(source, modality)?;
    let bytes = fs::read(source).map_err(|error| {
        media_error(
            "CALYX_MEDIA_SOURCE_READ_FAILED",
            format!("read source media {}: {error}", source.display()),
        )
    })?;
    validate_magic(&bytes, modality, &extension)?;
    let probe = probe::ffprobe_media(source, modality)?;
    let source_sha256 = sha256_hex(&bytes);
    let input_blake3 = *blake3::hash(&bytes).as_bytes();
    let rel = format!(
        "inputs/media/{}/{}.{}",
        modality_name(modality),
        source_sha256,
        extension
    );
    let pointer = format!("{INPUT_POINTER_PREFIX}{rel}");
    let retained_path = resolved.path.join(&rel);
    write_input_blob(&retained_path, &bytes)?;
    verify_retained_blob(&retained_path, &source_sha256, bytes.len())?;
    let mut metadata = media_metadata(&pointer, &source_sha256, bytes.len(), &extension, &probe);
    metadata.insert(
        "media.source_path".to_string(),
        source.display().to_string(),
    );
    Ok(RetainedMediaInput {
        input: Input::new(modality, bytes).with_pointer(pointer.clone()),
        metadata,
        pointer,
        source_sha256,
        input_blake3,
    })
}

mod derived;
mod probe;

fn validate_magic(bytes: &[u8], modality: Modality, extension: &str) -> ToolResult<()> {
    if bytes.is_empty() {
        return Err(media_error(
            "CALYX_MEDIA_EMPTY_INPUT",
            "media input is empty",
        ));
    }
    let ok = match (modality, extension) {
        (Modality::Image, "png") => {
            bytes.len() >= 8 && bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a])
        }
        (Modality::Image, "jpg" | "jpeg") => {
            bytes.len() >= 4 && bytes.starts_with(&[0xff, 0xd8, 0xff])
        }
        (Modality::Audio, "wav") => {
            bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WAVE"
        }
        (Modality::Video, "ogv") => bytes.starts_with(b"OggS"),
        (Modality::Video, "webm") => bytes.starts_with(&[0x1a, 0x45, 0xdf, 0xa3]),
        _ => false,
    };
    if ok {
        Ok(())
    } else {
        Err(media_error(
            "CALYX_MEDIA_MAGIC_MISMATCH",
            format!("{extension} bytes do not match expected {modality:?} container signature"),
        ))
    }
}

fn media_extension(source: &Path, modality: Modality) -> ToolResult<String> {
    let extension = source
        .extension()
        .and_then(|value| value.to_str())
        .map(|value| value.to_ascii_lowercase())
        .ok_or_else(|| {
            media_error(
                "CALYX_MEDIA_UNSUPPORTED_EXTENSION",
                format!("{} has no file extension", source.display()),
            )
        })?;
    let supported = match modality {
        Modality::Image => matches!(extension.as_str(), "png" | "jpg" | "jpeg"),
        Modality::Audio => extension == "wav",
        Modality::Video => matches!(extension.as_str(), "ogv" | "webm"),
        _ => false,
    };
    if supported {
        Ok(extension)
    } else {
        Err(media_error(
            "CALYX_MEDIA_UNSUPPORTED_EXTENSION",
            format!("unsupported {modality:?} media extension .{extension}"),
        ))
    }
}

fn verify_retained_blob(
    path: &Path,
    expected_sha256: &str,
    expected_bytes: usize,
) -> ToolResult<()> {
    let bytes = fs::read(path).map_err(|error| {
        media_error(
            "CALYX_MEDIA_RETAINED_BLOB_MISSING",
            format!("read retained media blob {}: {error}", path.display()),
        )
    })?;
    if bytes.len() != expected_bytes || sha256_hex(&bytes) != expected_sha256 {
        return Err(media_error(
            "CALYX_MEDIA_RETAINED_BLOB_MISMATCH",
            format!(
                "retained media blob {} did not read back intact",
                path.display()
            ),
        ));
    }
    Ok(())
}

fn media_metadata(
    pointer: &str,
    sha256: &str,
    bytes: usize,
    extension: &str,
    probe: &MediaProbe,
) -> BTreeMap<String, String> {
    let mut metadata = BTreeMap::new();
    metadata.insert("media.pointer".to_string(), pointer.to_string());
    metadata.insert("media.source_sha256".to_string(), sha256.to_string());
    metadata.insert("media.bytes".to_string(), bytes.to_string());
    metadata.insert("media.extension".to_string(), extension.to_string());
    metadata.insert("media.codec".to_string(), probe.codec.clone());
    metadata.insert("media.container".to_string(), probe.container.clone());
    optional_f64(
        &mut metadata,
        "media.duration_seconds",
        probe.duration_seconds,
    );
    optional_u32(&mut metadata, "media.sample_rate_hz", probe.sample_rate_hz);
    optional_u32(&mut metadata, "media.channels", probe.channels);
    optional_u32(&mut metadata, "media.width", probe.width);
    optional_u32(&mut metadata, "media.height", probe.height);
    if let Some(value) = probe.frame_count {
        metadata.insert("media.frame_count".to_string(), value.to_string());
    }
    optional_f64(&mut metadata, "media.fps", probe.fps);
    metadata
}

fn optional_u32(metadata: &mut BTreeMap<String, String>, key: &str, value: Option<u32>) {
    if let Some(value) = value {
        metadata.insert(key.to_string(), value.to_string());
    }
}

fn optional_f64(metadata: &mut BTreeMap<String, String>, key: &str, value: Option<f64>) {
    if let Some(value) = value {
        metadata.insert(key.to_string(), format!("{value:.6}"));
    }
}

fn incomplete_decode(source: &Path, media: &str) -> ToolError {
    media_error(
        "CALYX_MEDIA_DECODE_FAILED",
        format!("{} {media} metadata is incomplete", source.display()),
    )
}

fn ffprobe_codec_type(modality: Modality) -> &'static str {
    match modality {
        Modality::Image | Modality::Video => "video",
        Modality::Audio => "audio",
        _ => "media",
    }
}

fn modality_name(modality: Modality) -> &'static str {
    match modality {
        Modality::Image => "image",
        Modality::Audio => "audio",
        Modality::Video => "video",
        _ => "media",
    }
}

pub(super) fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub(super) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn media_error(code: &'static str, message: impl Into<String>) -> ToolError {
    CalyxError {
        code,
        message: message.into(),
        remediation: "inspect the media path, retained blob, ffprobe decode output, and Aster readback",
    }
    .into()
}

pub(super) fn retained_pointer_path(vault_dir: &Path, pointer: &str) -> ToolResult<PathBuf> {
    let Some(rel) = pointer.strip_prefix(INPUT_POINTER_PREFIX) else {
        return Err(media_error(
            "CALYX_MEDIA_POINTER_INVALID",
            format!("retained pointer {pointer:?} must start with {INPUT_POINTER_PREFIX}"),
        ));
    };
    let rel_path = Path::new(rel);
    if rel_path.components().any(|component| {
        matches!(
            component,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    }) {
        return Err(media_error(
            "CALYX_MEDIA_POINTER_INVALID",
            format!("retained pointer {pointer:?} escapes the vault"),
        ));
    }
    Ok(vault_dir.join(rel_path))
}

#[cfg(test)]
mod tests;
