use calyx_aster::vault::AsterVault;
use calyx_core::{Anchor, AnchorKind, AnchorValue, CalyxError, CxId};
use calyx_ledger::{ActorId, EntryKind, RedactionPolicy, SubjectId};
use serde_json::{Value, json};

use crate::server::{ToolError, ToolResult};

use super::DEFAULT_ANCHOR_SOURCE;

pub(super) fn append_anchor_ledger(
    vault: &AsterVault,
    cx_id: CxId,
    kind: &AnchorKind,
    anchor: Anchor,
) -> ToolResult<u64> {
    let bytes = serde_json::to_vec(&json!({
        "mode": "mcp-anchor",
        "anchor_kind": anchor_kind_key(kind),
    }))
    .map_err(|err| CalyxError::aster_corrupt_shard(format!("encode anchor ledger: {err}")))?;
    RedactionPolicy::check_payload(&bytes)?;
    Ok(vault
        .anchor_with_ledger_entry(
            cx_id,
            anchor,
            EntryKind::Ingest,
            SubjectId::Cx(cx_id),
            bytes,
            ActorId::Service(DEFAULT_ANCHOR_SOURCE.to_string()),
        )?
        .seq)
}

pub(super) fn parse_anchor_kind(value: &str, label: Option<&str>) -> ToolResult<AnchorKind> {
    Ok(match value {
        "test_pass" | "test-pass" => AnchorKind::TestPass,
        "thumbs_up" | "thumbs-up" | "thumbs_down" | "thumbs-down" => AnchorKind::Thumbs,
        "speaker_match" | "speaker-match" => AnchorKind::SpeakerMatch,
        "style_hold" | "style-hold" => AnchorKind::StyleHold,
        "label" => {
            let label = label.ok_or_else(|| {
                ToolError::invalid_params("anchor kind label requires label field")
            })?;
            if label.is_empty() {
                return Err(ToolError::invalid_params("anchor label must not be empty"));
            }
            AnchorKind::Label(label.to_string())
        }
        other => {
            return Err(ToolError::invalid_params(format!(
                "unknown anchor kind {other}"
            )));
        }
    })
}

pub(super) fn parse_anchor_value(kind: &str, value: &Value) -> ToolResult<AnchorValue> {
    match kind {
        "thumbs_up" | "thumbs-up" => return Ok(AnchorValue::Bool(true)),
        "thumbs_down" | "thumbs-down" => return Ok(AnchorValue::Bool(false)),
        _ => {}
    }
    if let Some(value) = value.as_bool() {
        return Ok(AnchorValue::Bool(value));
    }
    if let Some(value) = value.as_f64().filter(|value| value.is_finite()) {
        return Ok(AnchorValue::Number(value));
    }
    Err(ToolError::invalid_params(
        "anchor value must be a boolean or finite number",
    ))
}

pub(super) fn validate_confidence(value: f32) -> ToolResult<()> {
    if value.is_finite() && (0.0..=1.0).contains(&value) {
        return Ok(());
    }
    Err(ToolError::invalid_params(
        "confidence must be finite and within [0, 1]",
    ))
}

fn anchor_kind_key(kind: &AnchorKind) -> String {
    match kind {
        AnchorKind::TestPass => "test_pass".to_string(),
        AnchorKind::TieFormed => "tie_formed".to_string(),
        AnchorKind::Thumbs => "thumbs".to_string(),
        AnchorKind::Label(value) => format!("label:{value}"),
        AnchorKind::Reward => "reward".to_string(),
        AnchorKind::SpeakerMatch => "speaker_match".to_string(),
        AnchorKind::StyleHold => "style_hold".to_string(),
        AnchorKind::Recurrence => "recurrence".to_string(),
    }
}
