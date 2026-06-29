use std::collections::BTreeMap;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

use calyx_core::{CalyxError, Input, Modality, SlotState, VaultStore, media_modality_name};
use calyx_ledger::{ActorId, EntryKind, RedactionPolicy, SubjectId};
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
    append_ingest_retry_ledger, base_exists, decode, def, derived_text, enum_string,
    measure_constellation, open_vault, resolve_requested_vault,
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
        let reports = ingest_media_with_derived_text(&resolved, retained)?;
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
    let probe = ffprobe_media(source, modality)?;
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

fn ffprobe_media(source: &Path, modality: Modality) -> ToolResult<MediaProbe> {
    let codec_type = ffprobe_codec_type(modality);
    let mut command = Command::new("ffprobe");
    command.arg("-v").arg("error");
    if modality == Modality::Video {
        command.arg("-count_frames");
    }
    let output = command
        .arg("-show_streams")
        .arg("-show_format")
        .arg("-of")
        .arg("json")
        .arg(source)
        .output()
        .map_err(|error| {
            media_error(
                "CALYX_MEDIA_PROBE_MISSING",
                format!("spawn ffprobe for {}: {error}", source.display()),
            )
        })?;
    if !output.status.success() {
        return Err(media_error(
            "CALYX_MEDIA_DECODE_FAILED",
            format!(
                "ffprobe failed for {}: {}",
                source.display(),
                String::from_utf8_lossy(&output.stderr).trim()
            ),
        ));
    }
    let value: Value = serde_json::from_slice(&output.stdout).map_err(|error| {
        media_error(
            "CALYX_MEDIA_DECODE_FAILED",
            format!("parse ffprobe JSON for {}: {error}", source.display()),
        )
    })?;
    probe_from_json(&value, modality, codec_type, source)
}

fn probe_from_json(
    value: &Value,
    modality: Modality,
    codec_type: &str,
    source: &Path,
) -> ToolResult<MediaProbe> {
    let stream = value["streams"].as_array().and_then(|streams| {
        streams
            .iter()
            .find(|stream| stream["codec_type"].as_str() == Some(codec_type))
    });
    let Some(stream) = stream else {
        return Err(media_error(
            "CALYX_MEDIA_DECODE_FAILED",
            format!("{} has no {codec_type} stream", source.display()),
        ));
    };
    let container = value["format"]["format_name"]
        .as_str()
        .unwrap_or("")
        .to_string();
    let duration = stream["duration"]
        .as_str()
        .or_else(|| value["format"]["duration"].as_str())
        .and_then(|raw| raw.parse::<f64>().ok());
    let mut probe = MediaProbe {
        codec: stream["codec_name"].as_str().unwrap_or("").to_string(),
        container,
        duration_seconds: duration,
        sample_rate_hz: None,
        channels: None,
        width: None,
        height: None,
        frame_count: None,
        fps: None,
    };
    if modality == Modality::Audio {
        probe.sample_rate_hz = stream["sample_rate"]
            .as_str()
            .and_then(|raw| raw.parse::<u32>().ok());
        probe.channels = stream["channels"].as_u64().map(|value| value as u32);
        if probe.sample_rate_hz.unwrap_or(0) == 0 || probe.channels.unwrap_or(0) == 0 {
            return Err(incomplete_decode(source, "audio"));
        }
    } else {
        probe.width = stream["width"].as_u64().map(|value| value as u32);
        probe.height = stream["height"].as_u64().map(|value| value as u32);
        if probe.width.unwrap_or(0) == 0 || probe.height.unwrap_or(0) == 0 {
            return Err(incomplete_decode(source, media_modality_name(modality)));
        }
        if modality == Modality::Image {
            probe.frame_count = Some(1);
        } else {
            probe.frame_count = stream["nb_read_frames"]
                .as_str()
                .or_else(|| stream["nb_frames"].as_str())
                .and_then(|raw| raw.parse::<u64>().ok());
            probe.fps = stream["avg_frame_rate"]
                .as_str()
                .or_else(|| stream["r_frame_rate"].as_str())
                .and_then(parse_fps);
            if probe.frame_count.unwrap_or(0) == 0 || probe.fps.unwrap_or(0.0) <= 0.0 {
                return Err(incomplete_decode(source, "video"));
            }
        }
    }
    Ok(probe)
}

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

fn parse_fps(raw: &str) -> Option<f64> {
    let Some((left, right)) = raw.split_once('/') else {
        return raw.parse::<f64>().ok();
    };
    let numerator = left.parse::<f64>().ok()?;
    let denominator = right.parse::<f64>().ok()?;
    (denominator != 0.0).then_some(numerator / denominator)
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

fn ingest_media_with_derived_text(
    resolved: &ResolvedVault,
    retained: RetainedMediaInput,
) -> ToolResult<Vec<super::IngestReport>> {
    let vault = open_vault(resolved)?;
    let state = calyx_registry::load_vault_panel_state(&resolved.path)?;
    ensure_raw_media_panel_route(retained.input.modality, &state)?;
    let source_cx_id = vault.cx_id_for_input(&retained.input.bytes, state.panel.version);
    let derived = derived_text::derive_text_for_media(&resolved.path, &retained, source_cx_id)?;

    let mut media =
        measure_constellation(&vault, &state, retained.input.clone(), now_ms())?.constellation;
    media.metadata = retained.metadata.clone();
    let mut text =
        measure_constellation(&vault, &state, derived.input.clone(), now_ms())?.constellation;
    text.metadata = derived.metadata.clone();

    let media_new = !base_exists(&vault, media.cx_id)?;
    let text_new = !base_exists(&vault, text.cx_id)?;
    let payload =
        derived_text::derivation_ledger_payload(&retained, &derived, media.cx_id, text.cx_id)?;
    let mut staged = Vec::with_capacity(2);
    if media_new {
        staged.push(media.clone());
    }
    if text_new && text.cx_id != media.cx_id {
        staged.push(text.clone());
    }
    let explicit_derivation_seq = if staged.is_empty() {
        RedactionPolicy::check_payload(&payload)?;
        Some(
            vault
                .append_ledger_entry(
                    EntryKind::Ingest,
                    SubjectId::Cx(text.cx_id),
                    payload.clone(),
                    ActorId::Service("calyx-mcp".to_string()),
                )?
                .seq,
        )
    } else {
        vault.put_batch_with_ingest_ledger(
            staged,
            SubjectId::Cx(text.cx_id),
            payload,
            ActorId::Service("calyx-mcp".to_string()),
        )?;
        None
    };
    vault.flush()?;
    let snapshot = vault.snapshot();
    verify_media_readback(&vault, snapshot, &media, media_new)?;
    verify_media_readback(&vault, snapshot, &text, text_new)?;

    let media_seq = if media_new {
        vault.get(media.cx_id, snapshot)?.provenance.seq
    } else {
        append_ingest_retry_ledger(&vault, media.cx_id)?
    };
    let text_seq = if text_new {
        vault.get(text.cx_id, snapshot)?.provenance.seq
    } else if let Some(seq) = explicit_derivation_seq {
        seq
    } else {
        append_ingest_retry_ledger(&vault, text.cx_id)?
    };
    vault.flush()?;
    Ok(vec![
        super::IngestReport {
            cx_id: media.cx_id.to_string(),
            new: media_new,
            ledger_seq: media_seq,
        },
        super::IngestReport {
            cx_id: text.cx_id.to_string(),
            new: text_new,
            ledger_seq: text_seq,
        },
    ])
}

fn ensure_raw_media_panel_route(
    modality: Modality,
    state: &calyx_registry::VaultPanelState,
) -> ToolResult<()> {
    if !matches!(
        modality,
        Modality::Image | Modality::Audio | Modality::Video
    ) {
        return Ok(());
    }
    let has_declared_route = state
        .panel
        .slots
        .iter()
        .any(|slot| slot.state == SlotState::Active && slot.counts_toward_degraded(modality));
    if has_declared_route {
        return Ok(());
    }
    Err(CalyxError {
        code: "CALYX_MEDIA_ROUTE_UNAVAILABLE",
        message: format!(
            "raw {modality:?} ingest requires an active {modality:?} content lens before derived text can be attached"
        ),
        remediation:
            "add or activate an image/audio/video lens for the raw media modality, then re-run ingest so the media constellation is measured instead of empty",
    }
    .into())
}

fn verify_media_readback(
    vault: &calyx_aster::vault::AsterVault,
    snapshot: u64,
    expected: &calyx_core::Constellation,
    new: bool,
) -> ToolResult<()> {
    let stored = vault.get(expected.cx_id, snapshot)?;
    let mismatch = if new {
        stored.panel_version != expected.panel_version
            || stored.input_ref != expected.input_ref
            || stored.modality != expected.modality
            || stored.slots != expected.slots
            || stored.metadata != expected.metadata
            || stored.flags != expected.flags
    } else {
        stored.panel_version != expected.panel_version
            || stored.input_ref.hash != expected.input_ref.hash
            || stored.modality != expected.modality
            || stored.slots != expected.slots
    };
    if mismatch {
        return Err(CalyxError::aster_corrupt_shard(format!(
            "durable MCP media ingest readback mismatch for cx {}",
            expected.cx_id
        ))
        .into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsupported_video_extension_fails_closed() {
        let err = media_extension(Path::new("clip.txt"), Modality::Video).unwrap_err();
        assert!(format!("{err:?}").contains("CALYX_MEDIA_UNSUPPORTED_EXTENSION"));
    }

    #[test]
    fn wav_magic_is_checked_before_decode() {
        let err = validate_magic(b"not-wave", Modality::Audio, "wav").unwrap_err();
        assert!(format!("{err:?}").contains("CALYX_MEDIA_MAGIC_MISMATCH"));
    }
}
