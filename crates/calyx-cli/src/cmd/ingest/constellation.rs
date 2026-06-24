use std::collections::BTreeMap;

use calyx_aster::vault::AsterVault;
use calyx_core::{
    AbsentReason, Constellation, CxFlags, Input, InputRef, LedgerRef, LensId, Modality, Placement,
    SlotState, SlotVector,
};
use calyx_registry::VaultPanelState;
use rayon::prelude::*;

use crate::error::CliResult;

pub(crate) fn measure_constellation(
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

pub(crate) fn text_input(text: String) -> Input {
    Input::new(Modality::Text, text.into_bytes())
}

fn absent(reason: AbsentReason) -> SlotVector {
    SlotVector::Absent { reason }
}

fn input_hash(bytes: &[u8]) -> [u8; 32] {
    *blake3::hash(bytes).as_bytes()
}

/// Batch-measure a modality-uniform microbatch of inputs through every applicable
/// panel lens at once (one batched forward pass per lens), then assemble one
/// constellation per input from the readout. 10-50x faster than per-row measure
/// for GPU lenses; a degraded/broker-open lens yields an Absent slot (graceful).
pub(crate) fn measure_constellation_microbatch(
    vault: &AsterVault,
    state: &VaultPanelState,
    inputs: &[Input],
    now: u64,
) -> CliResult<Vec<Constellation>> {
    if inputs.is_empty() {
        return Ok(Vec::new());
    }
    let batch_modality = inputs[0].modality;
    // Partition applicable lenses by placement. GPU-CUDA lenses MUST run serially:
    // concurrent ONNX-CUDA Run() exhausts per-thread cuBLAS handles
    // (CUBLAS_STATUS_ALLOC_FAILED) and the CUDA EP single-streams anyway. CPU
    // lenses run in parallel and overlap the GPU work via rayon::join.
    let mut gpu_lenses: Vec<LensId> = Vec::new();
    let mut cpu_lenses: Vec<LensId> = Vec::new();
    for slot in &state.panel.slots {
        if slot.state == SlotState::Active
            && slot.modality == batch_modality
            && state.registry.contains(slot.lens_id)
        {
            match slot.resource.placement {
                Placement::Gpu => gpu_lenses.push(slot.lens_id),
                Placement::Cpu => cpu_lenses.push(slot.lens_id),
            }
        }
    }
    let measure_one = |lens_id: LensId| {
        state
            .registry
            .measure_batch(lens_id, inputs)
            .map(|vectors| (lens_id, vectors))
    };
    let (gpu_result, cpu_result) = rayon::join(
        || {
            gpu_lenses
                .iter()
                .map(|&id| measure_one(id))
                .collect::<std::result::Result<Vec<_>, _>>()
        },
        || {
            cpu_lenses
                .par_iter()
                .map(|&id| measure_one(id))
                .collect::<std::result::Result<Vec<_>, _>>()
        },
    );
    let mut measured: std::collections::BTreeMap<LensId, Vec<SlotVector>> =
        std::collections::BTreeMap::new();
    for (id, vectors) in gpu_result? {
        measured.insert(id, vectors);
    }
    for (id, vectors) in cpu_result? {
        measured.insert(id, vectors);
    }
    let mut out = Vec::with_capacity(inputs.len());
    for (i, input) in inputs.iter().enumerate() {
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
                match measured.get(&slot.lens_id) {
                    Some(vectors) if i < vectors.len() => vectors[i].clone(),
                    _ => absent(AbsentReason::LensUnavailable),
                }
            };
            degraded |= vector.is_absent();
            slots.insert(slot.slot_id, vector);
        }
        out.push(Constellation {
            cx_id: vault.cx_id_for_input(&input.bytes, state.panel.version),
            vault_id: vault.vault_id(),
            panel_version: state.panel.version,
            created_at: now,
            input_ref: InputRef {
                hash: input_hash(&input.bytes),
                pointer: input.pointer.clone(),
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
        });
    }
    Ok(out)
}
