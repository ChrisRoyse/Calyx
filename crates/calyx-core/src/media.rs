use crate::Modality;

pub const CALYX_MEDIA_DERIVED_TEXT_RUNTIME_MISSING: &str =
    "CALYX_MEDIA_DERIVED_TEXT_RUNTIME_MISSING";
pub const CALYX_MEDIA_DERIVED_TEXT_FAILED: &str = "CALYX_MEDIA_DERIVED_TEXT_FAILED";
pub const CALYX_MEDIA_DERIVED_TEXT_INVALID: &str = "CALYX_MEDIA_DERIVED_TEXT_INVALID";
pub const CALYX_MEDIA_ARTIFACT_INVALID: &str = "CALYX_MEDIA_ARTIFACT_INVALID";
pub const CALYX_MEDIA_ARTIFACT_COLLISION: &str = "CALYX_MEDIA_ARTIFACT_COLLISION";

pub const MEDIA_DERIVED_TEXT_ENV: &str = "CALYX_MEDIA_DERIVED_TEXT_CMD";

pub const DERIVED_TEXT_MODE: &str = "media-derived-text";
pub const DERIVED_KIND_TRANSCRIPT: &str = "transcript";
pub const DERIVED_KIND_CAPTION: &str = "caption";

pub const METADATA_DERIVED_KIND: &str = "derived.kind";
pub const METADATA_DERIVED_POINTER: &str = "derived.pointer";
pub const METADATA_DERIVED_TEXT_SHA256: &str = "derived.text_sha256";
pub const METADATA_DERIVED_TEXT_BYTES: &str = "derived.text_bytes";
pub const METADATA_DERIVED_RUNTIME: &str = "derived.runtime";
pub const METADATA_DERIVED_MODEL: &str = "derived.model";
pub const METADATA_DERIVED_LANGUAGE: &str = "derived.language";
pub const METADATA_DERIVED_CONFIDENCE: &str = "derived.confidence";
pub const METADATA_DERIVED_SOURCE_CX_ID: &str = "derived.source_cx_id";
pub const METADATA_DERIVED_SOURCE_MODALITY: &str = "derived.source_modality";
pub const METADATA_DERIVED_SOURCE_INPUT_HASH: &str = "derived.source_input_hash";
pub const METADATA_DERIVED_SOURCE_POINTER: &str = "derived.source_pointer";
pub const METADATA_DERIVED_SOURCE_SHA256: &str = "derived.source_sha256";

pub const LEDGER_FIELD_MODE: &str = "mode";
pub const LEDGER_FIELD_DERIVED_ARTIFACT_ID: &str = "derived_artifact_id";
pub const LEDGER_FIELD_SOURCE_CX_ID: &str = "source_cx_id";
pub const LEDGER_FIELD_TARGET_CX_ID: &str = "target_cx_id";
pub const LEDGER_FIELD_DERIVED_KIND: &str = "derived_kind";
pub const LEDGER_FIELD_SOURCE_MODALITY: &str = "source_modality";
pub const LEDGER_FIELD_SOURCE_INPUT_HASH: &str = "source_input_hash";
pub const LEDGER_FIELD_SOURCE_POINTER: &str = "source_pointer";
pub const LEDGER_FIELD_SOURCE_SHA256: &str = "source_sha256";
pub const LEDGER_FIELD_TARGET_POINTER: &str = "target_pointer";
pub const LEDGER_FIELD_TARGET_TEXT_SHA256: &str = "target_text_sha256";
pub const LEDGER_FIELD_RUNTIME: &str = "runtime";
pub const LEDGER_FIELD_MODEL: &str = "model";
pub const LEDGER_FIELD_RUNTIME_ID: &str = "runtime_id";
pub const LEDGER_FIELD_MODEL_ID: &str = "model_id";

pub const fn media_modality_name(modality: Modality) -> &'static str {
    match modality {
        Modality::Image => "image",
        Modality::Audio => "audio",
        Modality::Video => "video",
        _ => "media",
    }
}

pub const fn required_derived_kind(modality: Modality) -> Option<&'static str> {
    match modality {
        Modality::Audio | Modality::Video => Some(DERIVED_KIND_TRANSCRIPT),
        Modality::Image => Some(DERIVED_KIND_CAPTION),
        _ => None,
    }
}
