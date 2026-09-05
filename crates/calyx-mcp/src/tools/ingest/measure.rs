use std::collections::BTreeMap;

use calyx_aster::vault::AsterVault;
use calyx_core::{
    AbsentReason, CalyxError, Constellation, CxFlags, Input, InputRef, LedgerRef, Slot, SlotState,
    SlotVector,
};
use calyx_registry::VaultPanelState;

use super::input_retention::input_hash;
use crate::server::{ToolError, ToolResult};

pub(super) struct MeasuredConstellation {
    pub(super) constellation: Constellation,
}

pub(super) fn measure_constellation(
    vault: &AsterVault,
    state: &VaultPanelState,
    input: Input,
    now: u64,
) -> ToolResult<MeasuredConstellation> {
    let cx_id = {
        let identity = input.identity_bytes()?;
        vault.cx_id_for_input(identity.as_ref(), state.panel.version)
    };
    let mut slots = BTreeMap::new();
    let mut degraded = false;
    let mut applicable = 0usize;
    let mut produced = 0usize;
    let mut unavailable = 0usize;
    for slot in &state.panel.slots {
        let vector = measure_slot(
            slot,
            state,
            &input,
            &mut applicable,
            &mut produced,
            &mut unavailable,
        )?;
        degraded |= slot.counts_toward_degraded_for(input.modality, input.structured().is_some())
            && vector.is_absent();
        slots.insert(slot.slot_id, vector);
    }
    if applicable == 0 {
        return Err(ToolError::invalid_params(format!(
            "panel has no active {:?}-compatible slots",
            input.modality
        )));
    }
    if produced == 0 && unavailable == applicable {
        return Err(
            CalyxError::lens_unreachable("all applicable lens runtimes unreachable").into(),
        );
    }
    let mut scalars = BTreeMap::new();
    let mut metadata = BTreeMap::new();
    input.persist_structured(&mut scalars, &mut metadata)?;
    Ok(MeasuredConstellation {
        constellation: Constellation {
            cx_id,
            vault_id: vault.vault_id(),
            panel_version: state.panel.version,
            created_at: now,
            input_ref: InputRef {
                hash: input_hash(&input.bytes),
                pointer: input.pointer,
                redacted: false,
            },
            modality: input.modality,
            slots,
            scalars,
            metadata,
            anchors: Vec::new(),
            provenance: LedgerRef {
                seq: vault.latest_seq().saturating_add(1),
                hash: [0; 32],
            },
            flags: CxFlags {
                ungrounded: true,
                degraded,
                novel_region: false,
                redacted_input: false,
            },
        },
    })
}

fn measure_slot(
    slot: &Slot,
    state: &VaultPanelState,
    input: &Input,
    applicable: &mut usize,
    produced: &mut usize,
    unavailable: &mut usize,
) -> ToolResult<SlotVector> {
    if slot.state != SlotState::Active {
        return Ok(absent(AbsentReason::LensInactive));
    }
    if !state.registry.contains(slot.lens_id) && slot.modality == calyx_core::Modality::Structured {
        return Err(CalyxError::lens_unreachable(format!(
            "active structured slot {} references unregistered lens {}",
            slot.slot_id.get(),
            slot.lens_id
        ))
        .into());
    }
    if !state.registry.contains(slot.lens_id) {
        if input.supports_modality(slot.modality)
            || (slot.modality == calyx_core::Modality::Structured && input.structured().is_some())
        {
            *applicable += 1;
            *unavailable += 1;
            return Ok(absent(AbsentReason::LensUnavailable));
        }
        return Ok(absent(AbsentReason::NotApplicable));
    }
    let required_structured = state.registry.requires_structured_record(slot.lens_id)?;
    if !required_structured && !state.registry.accepts_input(slot.lens_id, input)? {
        return Ok(absent(AbsentReason::NotApplicable));
    }
    *applicable += 1;
    // A registered, Active lens that fails at runtime is an execution error, not a
    // load-state fact. Propagate the structured error verbatim (code/message/remediation)
    // so the whole request aborts with zero authority writes. Reclassifying a runtime
    // failure into `Absent(LensUnavailable)` (issue #1924/#1922) would silently persist a
    // degraded constellation. Absent-from-registry (a load-state fact) is handled above and
    // stays distinct.
    let vector = state.registry.measure(slot.lens_id, input)?;
    *produced += 1;
    Ok(vector)
}

fn absent(reason: AbsentReason) -> SlotVector {
    SlotVector::Absent { reason }
}
