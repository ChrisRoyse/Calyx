use std::collections::BTreeMap;

use calyx_aster::vault::AsterVault;
use calyx_core::{
    AbsentReason, Constellation, CxFlags, Input, InputRef, LedgerRef, Modality, SlotState,
    SlotVector,
};
use calyx_registry::VaultPanelState;

use crate::error::CliResult;

pub(super) fn measure_constellation(
    vault: &AsterVault,
    state: &VaultPanelState,
    input: Input,
    now: u64,
) -> CliResult<Constellation> {
    let cx_id = vault.cx_id_for_input(&input.bytes, state.panel.version);
    let mut slots = BTreeMap::new();
    let mut degraded = false;
    for slot in &state.panel.slots {
        let vector = if slot.state != SlotState::Active {
            absent(AbsentReason::LensInactive)
        } else if slot.modality != input.modality {
            absent(AbsentReason::NotApplicable)
        } else if !state.registry.contains(slot.lens_id) {
            absent(AbsentReason::LensUnavailable)
        } else {
            state.registry.measure(slot.lens_id, &input)?
        };
        degraded |= vector.is_absent();
        slots.insert(slot.slot_id, vector);
    }
    Ok(Constellation {
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
        scalars: BTreeMap::new(),
        metadata: BTreeMap::new(),
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
    })
}

pub(super) fn text_input(text: String) -> Input {
    Input::new(Modality::Text, text.into_bytes())
}

fn absent(reason: AbsentReason) -> SlotVector {
    SlotVector::Absent { reason }
}

fn input_hash(bytes: &[u8]) -> [u8; 32] {
    *blake3::hash(bytes).as_bytes()
}
