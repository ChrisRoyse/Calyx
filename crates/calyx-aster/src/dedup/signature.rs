use calyx_core::{
    CalyxError, Constellation, CxId, GuardTauProfile, Panel, Result, Slot, SlotId, dense_cosine,
};
use serde::{Deserialize, Serialize};

use super::{EpochSecs, TctCosineConfig, cosine_passes_all_required};

pub const CALYX_RECURRENCE_SLOT_MISSING: &str = "CALYX_RECURRENCE_SLOT_MISSING";
const SAME_TIME_EPSILON: f32 = 1.0e-6;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum SignatureResult {
    RecurrenceSignature {
        same_action: CxId,
        new_time: EpochSecs,
    },
    NewContent,
    ContentMismatch,
    SameTime,
}

pub fn detect_recurrence_signature(
    new_cx: &Constellation,
    existing_cx: &Constellation,
    config: &TctCosineConfig,
    temporal_slot_ids: &[SlotId],
    guard_profile: Option<&dyn GuardTauProfile>,
    new_time: EpochSecs,
) -> Result<SignatureResult> {
    if cosine_passes_all_required(new_cx, existing_cx, config, guard_profile)?.is_none() {
        return Ok(SignatureResult::ContentMismatch);
    }
    if temporal_slot_ids.is_empty() {
        return event_time_signature(existing_cx, new_time);
    }
    let mut temporal_differs = false;
    for slot in temporal_slot_ids {
        let new_dense = required_temporal_dense(new_cx, *slot)?;
        let existing_dense = required_temporal_dense(existing_cx, *slot)?;
        let cosine = dense_cosine(new_dense, existing_dense).ok_or_else(|| {
            recurrence_slot_error(format!("temporal slot {slot} has an invalid dense vector"))
        })?;
        if cosine < 1.0 - SAME_TIME_EPSILON {
            temporal_differs = true;
        }
    }
    if temporal_differs {
        Ok(SignatureResult::RecurrenceSignature {
            same_action: existing_cx.cx_id,
            new_time,
        })
    } else {
        Ok(SignatureResult::SameTime)
    }
}

pub fn temporal_slot_ids_for_panel(panel: &Panel) -> Vec<SlotId> {
    panel
        .slots
        .iter()
        .filter(|slot| is_temporal_slot(slot))
        .map(|slot| slot.slot_id)
        .collect()
}

fn required_temporal_dense(cx: &Constellation, slot: SlotId) -> Result<&[f32]> {
    cx.slots
        .get(&slot)
        .and_then(|vector| vector.as_dense())
        .ok_or_else(|| {
            recurrence_slot_error(format!(
                "constellation {} is missing dense temporal slot {slot}",
                cx.cx_id
            ))
        })
}

pub(crate) fn is_temporal_slot(slot: &Slot) -> bool {
    temporal_axis(slot.slot_key.key()) || slot.axis.as_deref().is_some_and(temporal_axis)
}

fn temporal_axis(value: &str) -> bool {
    matches!(
        value,
        "E2_recency" | "E3_periodic" | "E4_positional" | "E4_sequence"
    ) || value.starts_with("E2_")
        || value.starts_with("E3_")
        || value.starts_with("E4_")
}

fn event_time_signature(
    existing_cx: &Constellation,
    new_time: EpochSecs,
) -> Result<SignatureResult> {
    if existing_cx.created_at == new_time.to_u64()? {
        Ok(SignatureResult::SameTime)
    } else {
        Ok(SignatureResult::RecurrenceSignature {
            same_action: existing_cx.cx_id,
            new_time,
        })
    }
}

fn recurrence_slot_error(message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: CALYX_RECURRENCE_SLOT_MISSING,
        message: message.into(),
        remediation: "provide dense E2/E3/E4 temporal slots for recurrence signature detection",
    }
}

#[cfg(test)]
#[path = "signature_tests.rs"]
mod tests;
