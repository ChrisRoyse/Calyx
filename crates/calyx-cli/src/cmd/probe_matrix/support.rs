use calyx_core::{Modality, Slot, SlotId, SlotState};
use calyx_lodestar::{LodestarError, ProbeMatrixLog};

use super::persist::PersistedProbeMatrix;
use crate::error::{CliError, CliResult};

pub(super) fn active_text_slots(slots: &[Slot]) -> CliResult<Vec<SlotId>> {
    let out = slots
        .iter()
        .filter(|slot| slot.state == SlotState::Active && slot.modality == Modality::Text)
        .map(|slot| slot.slot_id)
        .collect::<Vec<_>>();
    if out.is_empty() {
        return Err(CliError::usage(
            "probe-matrix found no active text slots; pass --slot only after adding active text lenses",
        ));
    }
    Ok(out)
}

pub(super) fn validate_requested_slots(
    requested: &[SlotId],
    slots: &[Slot],
) -> CliResult<Vec<SlotId>> {
    for slot_id in requested {
        let Some(slot) = slots.iter().find(|slot| slot.slot_id == *slot_id) else {
            return Err(CliError::usage(format!(
                "--slot {slot_id} is not present in the vault panel"
            )));
        };
        if slot.state != SlotState::Active || slot.modality != Modality::Text {
            return Err(CliError::usage(format!(
                "--slot {slot_id} is not an active text slot"
            )));
        }
    }
    Ok(requested.to_vec())
}

pub(super) fn accepted_hit_count(log: &ProbeMatrixLog) -> usize {
    log.records
        .iter()
        .map(|record| record.accepted_hit_count)
        .sum()
}

pub(super) fn refusal_count(log: &ProbeMatrixLog) -> usize {
    log.records.iter().map(|record| record.refusals.len()).sum()
}

pub(super) fn invalid_params(detail: impl Into<String>) -> CliError {
    LodestarError::KernelInvalidParams {
        detail: detail.into(),
    }
    .into()
}

pub(super) fn with_persisted_artifact_error(
    error: CliError,
    persisted: &PersistedProbeMatrix,
) -> CliError {
    let detail = format!(
        "{}; diagnostic matrix artifact persisted at {} sha256={} records={} accepted_hits={} refusals={}",
        error.message(),
        persisted.path.display(),
        persisted.sha256,
        persisted.readback_record_count,
        persisted.readback_accepted_hit_count,
        persisted.readback_refusal_count
    );
    match error {
        CliError::Calyx(mut calyx) => {
            calyx.message = detail;
            CliError::Calyx(calyx)
        }
        CliError::Io(_) => CliError::io(detail),
        CliError::Usage(_) => CliError::usage(detail),
    }
}

pub(super) fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0f) as usize] as char);
    }
    out
}
