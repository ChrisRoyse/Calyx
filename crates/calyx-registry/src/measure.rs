//! Shared constellation measurement: run an input through a vault's panel and
//! assemble the per-lens [`Constellation`] (no-flatten — each lens keeps its own
//! slot). Used by the CLI ingest/measure paths and the read-only web API
//! `/v1/measure` endpoint so the two produce byte-identical constellations.

use std::collections::BTreeMap;

use calyx_aster::vault::AsterVault;
use calyx_core::{
    AbsentReason, Constellation, CxFlags, Input, InputRef, LedgerRef, Result, SlotState, SlotVector,
};

use crate::VaultPanelState;

/// Measure `input` through every slot of `state.panel`, returning the assembled
/// constellation. A slot is absent when its lens is inactive, inapplicable to
/// the input modality, or unregistered; otherwise the registered lens measures
/// it. The `degraded` flag is set when any degraded-counting slot is absent. The
/// caller decides whether a degraded constellation may be persisted (the ingest
/// path refuses an all-absent one); measurement itself never fails closed on
/// degradation so callers can inspect the real per-lens state.
pub fn measure_constellation(
    vault: &AsterVault,
    state: &VaultPanelState,
    input: Input,
    now: u64,
) -> Result<Constellation> {
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
        degraded |= slot.counts_toward_degraded(input.modality) && vector.is_absent();
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

/// An absent slot vector for `reason`.
pub fn absent(reason: AbsentReason) -> SlotVector {
    SlotVector::Absent { reason }
}

/// Blake3 of the raw input bytes, used as the constellation `InputRef.hash`.
pub fn input_hash(bytes: &[u8]) -> [u8; 32] {
    *blake3::hash(bytes).as_bytes()
}
