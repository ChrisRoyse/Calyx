use std::collections::BTreeMap;
use std::env;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

use calyx_aster::media_artifact::DerivedMediaArtifactDraft;
use calyx_core::{
    CALYX_MEDIA_DERIVED_TEXT_FAILED, CALYX_MEDIA_DERIVED_TEXT_INVALID,
    CALYX_MEDIA_DERIVED_TEXT_RUNTIME_MISSING, CalyxError, CxId, DERIVED_TEXT_MODE, Input,
    LEDGER_FIELD_DERIVED_ARTIFACT_ID, LEDGER_FIELD_DERIVED_KIND, LEDGER_FIELD_MODE,
    LEDGER_FIELD_MODEL_ID, LEDGER_FIELD_RUNTIME_ID, LEDGER_FIELD_SOURCE_CX_ID,
    LEDGER_FIELD_SOURCE_INPUT_HASH, LEDGER_FIELD_SOURCE_MODALITY, LEDGER_FIELD_SOURCE_SHA256,
    LEDGER_FIELD_TARGET_CX_ID, LEDGER_FIELD_TARGET_TEXT_SHA256, MEDIA_DERIVED_TEXT_ENV,
    METADATA_DERIVED_CONFIDENCE, METADATA_DERIVED_KIND, METADATA_DERIVED_LANGUAGE,
    METADATA_DERIVED_MODEL, METADATA_DERIVED_POINTER, METADATA_DERIVED_RUNTIME,
    METADATA_DERIVED_TEXT_BYTES, METADATA_DERIVED_TEXT_SHA256, Modality, media_modality_name,
    required_derived_kind,
};
use serde::Deserialize;
use serde_json::json;
use ulid::Ulid;

use crate::server::ToolResult;

use super::input_retention::INPUT_POINTER_PREFIX;
use super::media::{RetainedMediaInput, hex, retained_pointer_path, sha256_hex};

#[derive(Clone, Debug)]
pub(super) struct DerivedTextArtifact {
    pub(super) artifact_id: String,
    pub(super) input: Input,
    pub(super) metadata: BTreeMap<String, String>,
    pub(super) pointer: String,
    pub(super) text_sha256: String,
    pub(super) kind: &'static str,
    pub(super) runtime: String,
    pub(super) model: String,
    pub(super) language: Option<String>,
    pub(super) confidence: Option<f64>,
}

#[derive(Deserialize)]
struct DerivedTextOutput {
    text: String,
    runtime: String,
    model: String,
    language: Option<String>,
    confidence: Option<f64>,
}

pub(super) fn derive_text_for_media(
    vault_dir: &Path,
    retained: &RetainedMediaInput,
    source_cx_id: CxId,
) -> ToolResult<DerivedTextArtifact> {
    let kind = required_derived_kind(retained.input.modality).ok_or_else(|| {
        derived_error(
            CALYX_MEDIA_DERIVED_TEXT_INVALID,
            format!(
                "raw modality {:?} does not have a configured derived-text route",
                retained.input.modality
            ),
        )
    })?;
    let source_path = retained_pointer_path(vault_dir, &retained.pointer)?;
    let output_path = derived_command_output_path(vault_dir)?;
    let command_path = derived_command_path()?;
    eprintln!(
        "CALYX_MEDIA_DERIVED_TEXT phase=run_start surface=mcp cmd={} modality={} kind={} input={} output={}",
        command_path.display(),
        media_modality_name(retained.input.modality),
        kind,
        source_path.display(),
        output_path.display()
    );
    let output = Command::new(&command_path)
        .arg("--input")
        .arg(&source_path)
        .arg("--output")
        .arg(&output_path)
        .arg("--modality")
        .arg(media_modality_name(retained.input.modality))
        .arg("--kind")
        .arg(kind)
        .output()
        .map_err(|error| {
            derived_error(
                CALYX_MEDIA_DERIVED_TEXT_RUNTIME_MISSING,
                format!(
                    "spawn {} from {} failed: {error}",
                    MEDIA_DERIVED_TEXT_ENV,
                    command_path.display()
                ),
            )
        })?;
    if !output.status.success() {
        return Err(derived_error(
            CALYX_MEDIA_DERIVED_TEXT_FAILED,
            format!(
                "derived-text command {} exited {:?}; stdout={}; stderr={}",
                command_path.display(),
                output.status.code(),
                log_snippet(&output.stdout),
                log_snippet(&output.stderr)
            ),
        )
        .into());
    }
    let decoded = read_derived_output(&output_path)?;
    let language = decoded.language.filter(|value| !value.is_empty());
    let confidence = decoded.confidence;
    let text_bytes = decoded.text.as_bytes().to_vec();
    let text_sha256 = sha256_hex(&text_bytes);
    let artifact_id = Ulid::new().to_string();
    let rel = format!("inputs/derived_text/{kind}/{text_sha256}.txt");
    let pointer = format!("{INPUT_POINTER_PREFIX}{rel}");
    let retained_path = vault_dir.join(&rel);
    write_derived_text_blob(&retained_path, &text_bytes)?;
    verify_derived_text_pointer(vault_dir, &pointer, &text_sha256, text_bytes.len())?;

    let mut metadata = BTreeMap::new();
    metadata.insert(METADATA_DERIVED_KIND.to_string(), kind.to_string());
    metadata.insert(METADATA_DERIVED_POINTER.to_string(), pointer.clone());
    metadata.insert(
        METADATA_DERIVED_TEXT_SHA256.to_string(),
        text_sha256.clone(),
    );
    metadata.insert(
        METADATA_DERIVED_TEXT_BYTES.to_string(),
        text_bytes.len().to_string(),
    );
    metadata.insert(
        METADATA_DERIVED_RUNTIME.to_string(),
        decoded.runtime.clone(),
    );
    metadata.insert(METADATA_DERIVED_MODEL.to_string(), decoded.model.clone());
    if let Some(language) = language.as_ref() {
        metadata.insert(METADATA_DERIVED_LANGUAGE.to_string(), language.clone());
    }
    if let Some(confidence) = confidence {
        metadata.insert(
            METADATA_DERIVED_CONFIDENCE.to_string(),
            format!("{confidence:.6}"),
        );
    }
    eprintln!(
        "CALYX_MEDIA_DERIVED_TEXT phase=run_ok surface=mcp source_cx_id={} kind={} text_sha256={} text_bytes={}",
        source_cx_id,
        kind,
        text_sha256,
        text_bytes.len()
    );
    Ok(DerivedTextArtifact {
        artifact_id,
        input: Input::new(Modality::Text, text_bytes).with_pointer(pointer.clone()),
        metadata,
        pointer,
        text_sha256,
        kind,
        runtime: decoded.runtime,
        model: decoded.model,
        language,
        confidence,
    })
}

pub(super) fn derivation_ledger_payload(
    retained: &RetainedMediaInput,
    derived: &DerivedTextArtifact,
    source_cx_id: CxId,
    target_cx_id: CxId,
) -> ToolResult<Vec<u8>> {
    let mut payload = serde_json::Map::new();
    payload.insert(LEDGER_FIELD_MODE.to_string(), json!(DERIVED_TEXT_MODE));
    payload.insert(
        LEDGER_FIELD_DERIVED_ARTIFACT_ID.to_string(),
        json!(derived.artifact_id),
    );
    payload.insert(
        LEDGER_FIELD_SOURCE_CX_ID.to_string(),
        json!(source_cx_id.to_string()),
    );
    payload.insert(
        LEDGER_FIELD_TARGET_CX_ID.to_string(),
        json!(target_cx_id.to_string()),
    );
    payload.insert(LEDGER_FIELD_DERIVED_KIND.to_string(), json!(derived.kind));
    payload.insert(
        LEDGER_FIELD_SOURCE_MODALITY.to_string(),
        json!(media_modality_name(retained.input.modality)),
    );
    payload.insert(
        LEDGER_FIELD_SOURCE_INPUT_HASH.to_string(),
        json!(hex(&retained.input_blake3)),
    );
    payload.insert(
        LEDGER_FIELD_SOURCE_SHA256.to_string(),
        json!(retained.source_sha256),
    );
    payload.insert(
        LEDGER_FIELD_TARGET_TEXT_SHA256.to_string(),
        json!(derived.text_sha256),
    );
    payload.insert(
        LEDGER_FIELD_RUNTIME_ID.to_string(),
        json!(sha256_hex(derived.runtime.as_bytes())),
    );
    payload.insert(
        LEDGER_FIELD_MODEL_ID.to_string(),
        json!(sha256_hex(derived.model.as_bytes())),
    );
    serde_json::to_vec(&serde_json::Value::Object(payload)).map_err(|error| {
        derived_error(
            CALYX_MEDIA_DERIVED_TEXT_INVALID,
            format!("encode derived-text ledger payload: {error}"),
        )
        .into()
    })
}

pub(super) fn derived_artifact_draft(
    retained: &RetainedMediaInput,
    derived: &DerivedTextArtifact,
    source_cx_id: CxId,
    target_cx_id: CxId,
) -> ToolResult<DerivedMediaArtifactDraft> {
    Ok(DerivedMediaArtifactDraft {
        artifact_id: derived.artifact_id.clone(),
        source_cx_id,
        target_cx_id,
        derived_kind: derived.kind.to_string(),
        source_modality: media_modality_name(retained.input.modality).to_string(),
        source_input_hash: hex(&retained.input_blake3),
        source_sha256: retained.source_sha256.clone(),
        source_pointer: retained.pointer.clone(),
        target_pointer: derived.pointer.clone(),
        target_text_sha256: derived.text_sha256.clone(),
        runtime: derived.runtime.clone(),
        model: derived.model.clone(),
        language: derived.language.clone(),
        confidence: derived.confidence,
    })
}

fn derived_command_path() -> ToolResult<PathBuf> {
    let value = env::var_os(MEDIA_DERIVED_TEXT_ENV).ok_or_else(|| {
        derived_error(
            CALYX_MEDIA_DERIVED_TEXT_RUNTIME_MISSING,
            format!(
                "{MEDIA_DERIVED_TEXT_ENV} is required for raw image/audio/video ingest; configure an executable that writes transcript/caption JSON"
            ),
        )
    })?;
    if value.as_os_str().is_empty() {
        return Err(derived_error(
            CALYX_MEDIA_DERIVED_TEXT_RUNTIME_MISSING,
            format!("{MEDIA_DERIVED_TEXT_ENV} is empty"),
        )
        .into());
    }
    Ok(PathBuf::from(value))
}

fn derived_command_output_path(vault_dir: &Path) -> ToolResult<PathBuf> {
    let dir = vault_dir.join("tmp").join("derived_text");
    fs::create_dir_all(&dir).map_err(|error| {
        derived_error(
            CALYX_MEDIA_DERIVED_TEXT_FAILED,
            format!("create derived-text temp dir {}: {error}", dir.display()),
        )
    })?;
    Ok(dir.join(format!("{}.json", Ulid::new())))
}

fn read_derived_output(path: &Path) -> ToolResult<DerivedTextOutput> {
    let bytes = fs::read(path).map_err(|error| {
        derived_error(
            CALYX_MEDIA_DERIVED_TEXT_INVALID,
            format!("read derived-text output {}: {error}", path.display()),
        )
    })?;
    let decoded: DerivedTextOutput = serde_json::from_slice(&bytes).map_err(|error| {
        derived_error(
            CALYX_MEDIA_DERIVED_TEXT_INVALID,
            format!("parse derived-text output {}: {error}", path.display()),
        )
    })?;
    if decoded.text.trim().is_empty() {
        return Err(derived_error(
            CALYX_MEDIA_DERIVED_TEXT_INVALID,
            format!("derived-text output {} has empty text", path.display()),
        )
        .into());
    }
    if decoded.runtime.trim().is_empty() {
        return Err(derived_error(
            CALYX_MEDIA_DERIVED_TEXT_INVALID,
            format!("derived-text output {} has empty runtime", path.display()),
        )
        .into());
    }
    if decoded.model.trim().is_empty() {
        return Err(derived_error(
            CALYX_MEDIA_DERIVED_TEXT_INVALID,
            format!("derived-text output {} has empty model", path.display()),
        )
        .into());
    }
    if let Some(confidence) = decoded.confidence
        && (!confidence.is_finite() || !(0.0..=1.0).contains(&confidence))
    {
        return Err(derived_error(
            CALYX_MEDIA_DERIVED_TEXT_INVALID,
            format!(
                "derived-text output {} confidence {confidence} is outside [0,1]",
                path.display()
            ),
        )
        .into());
    }
    Ok(decoded)
}

fn write_derived_text_blob(path: &Path, bytes: &[u8]) -> ToolResult<()> {
    if let Ok(existing) = fs::read(path) {
        if existing == bytes {
            return Ok(());
        }
        return Err(derived_error(
            CALYX_MEDIA_DERIVED_TEXT_INVALID,
            format!(
                "derived text blob {} exists with different bytes",
                path.display()
            ),
        )
        .into());
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            derived_error(
                CALYX_MEDIA_DERIVED_TEXT_FAILED,
                format!("create derived text dir {}: {error}", parent.display()),
            )
        })?;
    }
    let tmp = path.with_extension(format!(
        "{}.tmp-{}",
        path.extension()
            .and_then(|value| value.to_str())
            .unwrap_or("txt"),
        std::process::id()
    ));
    {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&tmp)
            .map_err(|error| {
                derived_error(
                    CALYX_MEDIA_DERIVED_TEXT_FAILED,
                    format!("create derived text temp {}: {error}", tmp.display()),
                )
            })?;
        file.write_all(bytes).map_err(|error| {
            derived_error(
                CALYX_MEDIA_DERIVED_TEXT_FAILED,
                format!("write derived text temp {}: {error}", tmp.display()),
            )
        })?;
        file.sync_all().map_err(|error| {
            derived_error(
                CALYX_MEDIA_DERIVED_TEXT_FAILED,
                format!("sync derived text temp {}: {error}", tmp.display()),
            )
        })?;
    }
    fs::rename(&tmp, path).map_err(|error| {
        derived_error(
            CALYX_MEDIA_DERIVED_TEXT_FAILED,
            format!("install derived text blob {}: {error}", path.display()),
        )
    })?;
    Ok(())
}

fn verify_derived_text_pointer(
    vault_dir: &Path,
    pointer: &str,
    expected_sha256: &str,
    expected_bytes: usize,
) -> ToolResult<()> {
    let path = retained_pointer_path(vault_dir, pointer)?;
    let bytes = fs::read(&path).map_err(|error| {
        derived_error(
            CALYX_MEDIA_DERIVED_TEXT_INVALID,
            format!("read derived text blob {}: {error}", path.display()),
        )
    })?;
    if bytes.len() != expected_bytes {
        return Err(derived_error(
            CALYX_MEDIA_DERIVED_TEXT_INVALID,
            format!(
                "derived text blob {} has {} bytes, expected {expected_bytes}",
                path.display(),
                bytes.len()
            ),
        )
        .into());
    }
    let actual = sha256_hex(&bytes);
    if actual != expected_sha256 {
        return Err(derived_error(
            CALYX_MEDIA_DERIVED_TEXT_INVALID,
            format!(
                "derived text blob {} sha256 {actual} != expected {expected_sha256}",
                path.display()
            ),
        )
        .into());
    }
    Ok(())
}

fn derived_error(code: &'static str, message: impl Into<String>) -> CalyxError {
    CalyxError {
        code,
        message: message.into(),
        remediation: "configure CALYX_MEDIA_DERIVED_TEXT_CMD to a real OCR/transcription executable that accepts --input <path> --output <json> --modality <image|audio|video> --kind <caption|transcript>, then inspect the derived-text JSON and Aster readback",
    }
}

fn log_snippet(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes).replace(['\r', '\n'], " ");
    let trimmed = text.trim();
    if trimmed.len() <= 2048 {
        trimmed.to_string()
    } else {
        format!("{}...", &trimmed[..2048])
    }
}
