use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

use calyx_core::{CalyxError, Input, Modality, Result};
use serde_json::Value;
use sha2::{Digest, Sha256};

const POINTER_PREFIX: &str = "calyx-vault://";

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct MediaProbe {
    pub(crate) codec: String,
    pub(crate) container: String,
    pub(crate) duration_seconds: Option<f64>,
    pub(crate) sample_rate_hz: Option<u32>,
    pub(crate) channels: Option<u32>,
    pub(crate) width: Option<u32>,
    pub(crate) height: Option<u32>,
    pub(crate) frame_count: Option<u64>,
    pub(crate) fps: Option<f64>,
}

#[derive(Clone, Debug)]
pub(crate) struct RetainedMediaInput {
    pub(crate) input: Input,
    pub(crate) pointer: String,
    pub(crate) source_sha256: String,
    pub(crate) input_blake3: [u8; 32],
    pub(crate) bytes: usize,
    pub(crate) extension: String,
    pub(crate) probe: MediaProbe,
}

pub(crate) fn parse_audio_video_modality(raw: &str) -> Result<Modality> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "audio" => Ok(Modality::Audio),
        "video" => Ok(Modality::Video),
        other => Err(media_error(
            "CALYX_MEDIA_UNSUPPORTED_MODALITY",
            format!("unsupported raw media modality {other}; expected audio or video"),
        )),
    }
}

pub(crate) fn retain_media_input(
    vault_dir: &Path,
    source: &Path,
    modality: Modality,
) -> Result<RetainedMediaInput> {
    ensure_supported_modality(modality)?;
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
    let byte_len = bytes.len();
    let input_blake3 = *blake3::hash(&bytes).as_bytes();
    let rel = format!(
        "inputs/media/{}/{}.{}",
        modality_name(modality),
        source_sha256,
        extension
    );
    let pointer = format!("{POINTER_PREFIX}{rel}");
    let retained_path = vault_dir.join(&rel);
    write_retained_blob(&retained_path, &bytes)?;
    verify_retained_pointer(vault_dir, &pointer, &source_sha256, bytes.len())?;
    Ok(RetainedMediaInput {
        input: Input::new(modality, bytes).with_pointer(pointer.clone()),
        pointer,
        source_sha256,
        input_blake3,
        bytes: byte_len,
        extension,
        probe,
    })
}

pub(crate) fn verify_retained_pointer(
    vault_dir: &Path,
    pointer: &str,
    expected_sha256: &str,
    expected_bytes: usize,
) -> Result<()> {
    let path = retained_pointer_path(vault_dir, pointer)?;
    let bytes = fs::read(&path).map_err(|error| {
        media_error(
            "CALYX_MEDIA_RETAINED_BLOB_MISSING",
            format!("read retained media blob {}: {error}", path.display()),
        )
    })?;
    if bytes.len() != expected_bytes {
        return Err(media_error(
            "CALYX_MEDIA_RETAINED_BLOB_MISMATCH",
            format!(
                "retained media blob {} has {} bytes, expected {expected_bytes}",
                path.display(),
                bytes.len()
            ),
        ));
    }
    let actual = sha256_hex(&bytes);
    if actual != expected_sha256 {
        return Err(media_error(
            "CALYX_MEDIA_RETAINED_BLOB_MISMATCH",
            format!(
                "retained media blob {} sha256 {actual} != expected {expected_sha256}",
                path.display()
            ),
        ));
    }
    Ok(())
}

pub(crate) fn retained_pointer_path(vault_dir: &Path, pointer: &str) -> Result<PathBuf> {
    let Some(rel) = pointer.strip_prefix(POINTER_PREFIX) else {
        return Err(media_error(
            "CALYX_MEDIA_POINTER_INVALID",
            format!("retained pointer {pointer:?} must start with {POINTER_PREFIX}"),
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

pub(crate) fn media_metadata(retained: &RetainedMediaInput) -> BTreeMap<String, String> {
    let mut metadata = BTreeMap::new();
    metadata.insert("media.pointer".to_string(), retained.pointer.clone());
    metadata.insert(
        "media.source_sha256".to_string(),
        retained.source_sha256.clone(),
    );
    metadata.insert("media.bytes".to_string(), retained.bytes.to_string());
    metadata.insert("media.extension".to_string(), retained.extension.clone());
    metadata.insert("media.codec".to_string(), retained.probe.codec.clone());
    metadata.insert(
        "media.container".to_string(),
        retained.probe.container.clone(),
    );
    if let Some(value) = retained.probe.duration_seconds {
        metadata.insert("media.duration_seconds".to_string(), format!("{value:.6}"));
    }
    if let Some(value) = retained.probe.sample_rate_hz {
        metadata.insert("media.sample_rate_hz".to_string(), value.to_string());
    }
    if let Some(value) = retained.probe.channels {
        metadata.insert("media.channels".to_string(), value.to_string());
    }
    if let Some(value) = retained.probe.frame_count {
        metadata.insert("media.frame_count".to_string(), value.to_string());
    }
    if let Some(value) = retained.probe.width {
        metadata.insert("media.width".to_string(), value.to_string());
    }
    if let Some(value) = retained.probe.height {
        metadata.insert("media.height".to_string(), value.to_string());
    }
    if let Some(value) = retained.probe.fps {
        metadata.insert("media.fps".to_string(), format!("{value:.6}"));
    }
    metadata
}

pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(hex_digit(byte >> 4));
        out.push(hex_digit(byte & 0x0f));
    }
    out
}

fn ffprobe_media(source: &Path, modality: Modality) -> Result<MediaProbe> {
    let codec_type = modality_name(modality);
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
    probe_from_json(&value, codec_type, source)
}

fn probe_from_json(value: &Value, codec_type: &str, source: &Path) -> Result<MediaProbe> {
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
    if codec_type == "audio" {
        probe.sample_rate_hz = stream["sample_rate"]
            .as_str()
            .and_then(|raw| raw.parse::<u32>().ok());
        probe.channels = stream["channels"].as_u64().map(|value| value as u32);
        if probe.sample_rate_hz.unwrap_or(0) == 0 || probe.channels.unwrap_or(0) == 0 {
            return Err(media_error(
                "CALYX_MEDIA_DECODE_FAILED",
                format!("{} audio metadata is incomplete", source.display()),
            ));
        }
    } else {
        probe.width = stream["width"].as_u64().map(|value| value as u32);
        probe.height = stream["height"].as_u64().map(|value| value as u32);
        probe.frame_count = stream["nb_read_frames"]
            .as_str()
            .or_else(|| stream["nb_frames"].as_str())
            .and_then(|raw| raw.parse::<u64>().ok());
        probe.fps = stream["avg_frame_rate"]
            .as_str()
            .or_else(|| stream["r_frame_rate"].as_str())
            .and_then(parse_fps);
        if probe.width.unwrap_or(0) == 0
            || probe.height.unwrap_or(0) == 0
            || probe.frame_count.unwrap_or(0) == 0
            || probe.fps.unwrap_or(0.0) <= 0.0
        {
            return Err(media_error(
                "CALYX_MEDIA_DECODE_FAILED",
                format!("{} video metadata is incomplete", source.display()),
            ));
        }
    }
    Ok(probe)
}

fn parse_fps(raw: &str) -> Option<f64> {
    let Some((left, right)) = raw.split_once('/') else {
        return raw.parse::<f64>().ok();
    };
    let numerator = left.parse::<f64>().ok()?;
    let denominator = right.parse::<f64>().ok()?;
    if denominator == 0.0 {
        None
    } else {
        Some(numerator / denominator)
    }
}

fn write_retained_blob(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Ok(existing) = fs::read(path) {
        if existing == bytes {
            return Ok(());
        }
        return Err(media_error(
            "CALYX_MEDIA_RETAINED_BLOB_CONFLICT",
            format!(
                "retained media blob {} exists with different bytes",
                path.display()
            ),
        ));
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            media_error(
                "CALYX_MEDIA_RETAINED_BLOB_WRITE_FAILED",
                format!("create retained media dir {}: {error}", parent.display()),
            )
        })?;
    }
    let tmp = path.with_extension(format!(
        "{}.tmp-{}",
        path.extension()
            .and_then(|value| value.to_str())
            .unwrap_or("bin"),
        std::process::id()
    ));
    {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&tmp)
            .map_err(|error| {
                media_error(
                    "CALYX_MEDIA_RETAINED_BLOB_WRITE_FAILED",
                    format!("create retained media temp {}: {error}", tmp.display()),
                )
            })?;
        file.write_all(bytes).map_err(|error| {
            media_error(
                "CALYX_MEDIA_RETAINED_BLOB_WRITE_FAILED",
                format!("write retained media temp {}: {error}", tmp.display()),
            )
        })?;
        file.sync_all().map_err(|error| {
            media_error(
                "CALYX_MEDIA_RETAINED_BLOB_WRITE_FAILED",
                format!("sync retained media temp {}: {error}", tmp.display()),
            )
        })?;
    }
    fs::rename(&tmp, path).map_err(|error| {
        media_error(
            "CALYX_MEDIA_RETAINED_BLOB_WRITE_FAILED",
            format!("install retained media blob {}: {error}", path.display()),
        )
    })
}

fn validate_magic(bytes: &[u8], modality: Modality, extension: &str) -> Result<()> {
    if bytes.is_empty() {
        return Err(media_error(
            "CALYX_MEDIA_EMPTY_INPUT",
            "media input is empty",
        ));
    }
    let ok = match (modality, extension) {
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

fn media_extension(source: &Path, modality: Modality) -> Result<String> {
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

fn ensure_supported_modality(modality: Modality) -> Result<()> {
    if matches!(modality, Modality::Audio | Modality::Video) {
        Ok(())
    } else {
        Err(media_error(
            "CALYX_MEDIA_UNSUPPORTED_MODALITY",
            format!("raw media ingest supports audio/video, got {modality:?}"),
        ))
    }
}

fn modality_name(modality: Modality) -> &'static str {
    match modality {
        Modality::Audio => "audio",
        Modality::Video => "video",
        _ => "media",
    }
}

fn media_error(code: &'static str, message: impl Into<String>) -> CalyxError {
    CalyxError {
        code,
        message: message.into(),
        remediation: "inspect the media path, retained blob, ffprobe decode output, and Aster readback",
    }
}

fn hex_digit(value: u8) -> char {
    match value {
        0..=9 => char::from(b'0' + value),
        10..=15 => char::from(b'a' + value - 10),
        _ => unreachable!("nibble out of range"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pointer_escape_is_rejected() {
        let err = retained_pointer_path(Path::new("/vault"), "calyx-vault://../x").unwrap_err();
        assert_eq!(err.code, "CALYX_MEDIA_POINTER_INVALID");
    }

    #[test]
    fn unsupported_video_extension_fails_closed() {
        let err = media_extension(Path::new("clip.txt"), Modality::Video).unwrap_err();
        assert_eq!(err.code, "CALYX_MEDIA_UNSUPPORTED_EXTENSION");
    }

    #[test]
    fn wav_magic_is_checked_before_decode() {
        let err = validate_magic(b"not-wave", Modality::Audio, "wav").unwrap_err();
        assert_eq!(err.code, "CALYX_MEDIA_MAGIC_MISMATCH");
    }
}
