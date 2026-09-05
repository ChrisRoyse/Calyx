use std::collections::BTreeMap;
use std::str::FromStr;
use std::time::Instant;

mod measurement;

use calyx_aster::vault::AsterVault;
use calyx_core::{
    AbsentReason, CalyxError, CalyxErrorCode, Constellation, CxFlags, Input, InputRef, LedgerRef,
    LensId, MeasurementGroupKey, Modality, Placement, SlotId, SlotState, SlotVector,
    StructuredRecord,
};
use calyx_registry::VaultPanelState;
pub(crate) use calyx_registry::measure::{absent, input_hash};
use rayon::prelude::*;

use super::command::ingest_runtime_log;
use super::route::{IngestGpuRoute, gpu_route_required_error};
use super::worker::measure_lens_in_worker;
use crate::error::CliResult;
use crate::lens_commands::support::runtime_name;
use crate::panel_commands::measure_resident_batch_at;
use measurement::*;

/// Doctrine #1273 rule 3 ("never single — fail hard"): an ingest that leaves
/// every declared, applicable content lens unmeasured would silently persist a
/// constellation weaker than the panel promises and yield illusory retrieval
/// (the search-returns-`[]` footgun). When EVERY content lens for the input
/// modality is absent we refuse to persist and name each absent slot + reason so
/// the operator can bind/repair the runtime. Partial degradation still records
/// the `degraded` flag (full panel-floor enforcement is tracked separately).
pub(crate) fn ensure_content_panel_floor(
    cx: &Constellation,
    state: &VaultPanelState,
) -> CliResult<()> {
    let has_structured = StructuredRecord::from_persisted(&cx.scalars, &cx.metadata)?.is_some();
    let mut declared = 0usize;
    let mut absent: Vec<String> = Vec::new();
    for slot in &state.panel.slots {
        if !slot.counts_toward_degraded_for(cx.modality, has_structured) {
            continue;
        }
        declared += 1;
        if let Some(SlotVector::Absent { reason }) = cx.slots.get(&slot.slot_id) {
            let spec = state.registry.lens_spec(slot.lens_id);
            let runtime = spec
                .map(|spec| runtime_name(&spec.runtime))
                .unwrap_or("unregistered");
            let spec_name = spec
                .map(|spec| spec.name.as_str())
                .unwrap_or("missing_registry_snapshot");
            absent.push(format!(
                "slot={} key={} lens={} spec_name={} runtime={} modality={:?} shape={:?} placement={:?} reason={:?}",
                slot.slot_id.get(),
                slot.slot_key.key(),
                slot.lens_id,
                spec_name,
                runtime,
                slot.modality,
                slot.shape,
                slot.resource.placement,
                reason
            ));
        }
    }
    if declared > 0 && absent.len() == declared {
        return Err(CalyxError::from_code(
            CalyxErrorCode::LensUnreachable,
            format!(
                "ingest refused for cx {:?}: 0/{} declared content lenses materialized for \
                 modality {:?} — every content lens is unavailable, so this constellation \
                 would be silently empty and unsearchable. Bind/repair the lens runtimes \
                 (calyx add-lens / verify runtime endpoints) and re-ingest. Absent content \
                 slots: [{}]",
                cx.cx_id,
                declared,
                cx.modality,
                absent.join("; ")
            ),
        )
        .into());
    }
    Ok(())
}

pub(crate) fn text_input(text: String) -> Input {
    Input::new(Modality::Text, text.into_bytes())
}

/// Single-input measurement with cold GPU workers allowed: used by the
/// `calyx measure` debug command and in-crate tests, which are not the
/// batch-ingest surface gated by #1004.
pub(crate) fn measure_constellation(
    vault: &AsterVault,
    state: &VaultPanelState,
    input: Input,
    now: u64,
) -> CliResult<Constellation> {
    let mut measured =
        measure_constellation_microbatch(vault, state, std::slice::from_ref(&input), now)?;
    match measured.len() {
        1 => Ok(measured.remove(0)),
        count => Err(CalyxError::lens_dim_mismatch(format!(
            "single constellation measurement returned {count} constellations"
        ))
        .into()),
    }
}

pub(crate) fn measure_constellation_with_runtime_limit(
    vault: &AsterVault,
    state: &VaultPanelState,
    input: &Input,
    now: u64,
    runtime_batch_limit: Option<usize>,
    gpu_route: IngestGpuRoute,
) -> CliResult<Constellation> {
    let mut measured = measure_constellation_microbatch_with_runtime_limit(
        vault,
        state,
        std::slice::from_ref(input),
        now,
        runtime_batch_limit,
        gpu_route,
    )?;
    match measured.len() {
        1 => Ok(measured.remove(0)),
        count => Err(CalyxError::lens_dim_mismatch(format!(
            "single constellation measurement returned {count} constellations"
        ))
        .into()),
    }
}

#[derive(Clone, Copy)]
struct ApplicableLens {
    lens_id: LensId,
    slot_id: SlotId,
    placement: Placement,
}

struct ApplicableLensJob {
    lenses: Vec<ApplicableLens>,
    grouped: bool,
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
    measure_constellation_microbatch_with_runtime_limit(
        vault,
        state,
        inputs,
        now,
        None,
        IngestGpuRoute::cold_workers_allowed(),
    )
}

pub(crate) fn measure_constellation_microbatch_with_runtime_limit(
    vault: &AsterVault,
    state: &VaultPanelState,
    inputs: &[Input],
    now: u64,
    runtime_batch_limit: Option<usize>,
    gpu_route: IngestGpuRoute,
) -> CliResult<Vec<Constellation>> {
    if inputs.is_empty() {
        return Ok(Vec::new());
    }
    let started = Instant::now();
    let batch_modality = inputs[0].modality;
    for (index, input) in inputs.iter().enumerate().skip(1) {
        if input.modality != batch_modality {
            return Err(CalyxError::lens_dim_mismatch(format!(
                "measure microbatch requires uniform modality: input 0 is {:?}, input {index} is {:?}",
                batch_modality, input.modality
            ))
            .into());
        }
    }
    // Partition applicable lenses by placement. GPU-CUDA lenses MUST run serially:
    // concurrent ONNX-CUDA Run() exhausts per-thread cuBLAS handles
    // (CUBLAS_STATUS_ALLOC_FAILED) and the CUDA EP single-streams anyway. CPU
    // lenses run in parallel and overlap the GPU work via rayon::join.
    let mut gpu_lenses: Vec<ApplicableLens> = Vec::new();
    let mut cpu_lenses: Vec<ApplicableLens> = Vec::new();
    for slot in &state.panel.slots {
        if slot.state != SlotState::Active {
            continue;
        }
        if !state.registry.contains(slot.lens_id) {
            // A missing *content* structured lens is real signal loss and stays
            // fail-closed. A compute-on-recall sidecar has no frozen model by
            // design and is recorded NotApplicable below (#1947).
            if slot.modality == Modality::Structured && !slot.is_compute_on_recall_sidecar() {
                return Err(CalyxError::lens_unreachable(format!(
                    "active structured slot {} references unregistered lens {}",
                    slot.slot_id.get(),
                    slot.lens_id
                ))
                .into());
            }
            continue;
        }
        let requires_structured = state.registry.requires_structured_record(slot.lens_id)?;
        let applicable = requires_structured
            || inputs
                .iter()
                .map(|input| state.registry.accepts_input(slot.lens_id, input))
                .collect::<calyx_core::Result<Vec<_>>>()?
                .into_iter()
                .all(|accepts| accepts);
        if !applicable {
            continue;
        }
        let lens = ApplicableLens {
            lens_id: slot.lens_id,
            slot_id: slot.slot_id,
            placement: slot.resource.placement,
        };
        match slot.resource.placement {
            Placement::Gpu => gpu_lenses.push(lens),
            Placement::Cpu => cpu_lenses.push(lens),
        }
    }
    ingest_runtime_log(format_args!(
        "phase=measure_microbatch_start modality={:?} batch_size={} gpu_lenses={} cpu_lenses={} runtime_batch_limit={:?} resident_addr={:?}",
        batch_modality,
        inputs.len(),
        gpu_lenses.len(),
        cpu_lenses.len(),
        runtime_batch_limit,
        gpu_route.resident_addr
    ));
    let (gpu_vectors, cpu_vectors) = if let Some(addr) = gpu_route.resident_addr {
        if !gpu_lenses.is_empty() {
            ingest_runtime_log(format_args!(
                "phase=measure_resident_service_gate addr={} gpu_lenses={} local_lenses_deferred=true",
                addr,
                gpu_lenses.len()
            ));
        }
        let gpu_vectors = resident_batch::measure_gpu_lenses_via_resident_service(
            state,
            &gpu_lenses,
            batch_modality,
            inputs,
            runtime_batch_limit,
            addr,
        )?;
        let cpu_jobs = group_applicable_lenses(state, &cpu_lenses)?;
        let cpu_vectors: Vec<(LensId, Vec<SlotVector>)> = cpu_jobs
            .par_iter()
            .map(|job| {
                measure_applicable_lens_job(state, job, batch_modality, inputs, runtime_batch_limit)
            })
            .collect::<std::result::Result<Vec<_>, _>>()?
            .into_iter()
            .flatten()
            .collect();
        (gpu_vectors, cpu_vectors)
    } else {
        // #1004 fail-closed gate: active GPU lenses without a resident route
        // must not silently take the cold per-invocation worker path (full
        // model reload per lens per command — the #999 slow path).
        if !gpu_lenses.is_empty() && !gpu_route.allow_cold_gpu_workers {
            return Err(
                gpu_route_required_error(gpu_lenses.len(), batch_modality, gpu_route).into(),
            );
        }
        let gpu_jobs = group_applicable_lenses(state, &gpu_lenses)?;
        let cpu_jobs = group_applicable_lenses(state, &cpu_lenses)?;
        let (gpu_result, cpu_result) = rayon::join(
            || {
                gpu_jobs
                    .iter()
                    .map(|job| {
                        measure_applicable_lens_job(
                            state,
                            job,
                            batch_modality,
                            inputs,
                            runtime_batch_limit,
                        )
                    })
                    .collect::<std::result::Result<Vec<_>, _>>()
                    .map(|groups| {
                        groups
                            .into_iter()
                            .flatten()
                            .collect::<Vec<(LensId, Vec<SlotVector>)>>()
                    })
            },
            || {
                cpu_jobs
                    .par_iter()
                    .map(|job| {
                        measure_applicable_lens_job(
                            state,
                            job,
                            batch_modality,
                            inputs,
                            runtime_batch_limit,
                        )
                    })
                    .collect::<std::result::Result<Vec<_>, _>>()
                    .map(|groups| {
                        groups
                            .into_iter()
                            .flatten()
                            .collect::<Vec<(LensId, Vec<SlotVector>)>>()
                    })
            },
        );
        (gpu_result?, cpu_result?)
    };
    let mut measured: std::collections::BTreeMap<LensId, Vec<SlotVector>> =
        std::collections::BTreeMap::new();
    for (id, vectors) in gpu_vectors {
        measured.insert(id, vectors);
    }
    for (id, vectors) in cpu_vectors {
        measured.insert(id, vectors);
    }
    let mut out = Vec::with_capacity(inputs.len());
    for (i, input) in inputs.iter().enumerate() {
        let mut slots = BTreeMap::new();
        let mut degraded = false;
        for slot in &state.panel.slots {
            let vector = if slot.state != SlotState::Active {
                absent(AbsentReason::LensInactive)
            } else if !state.registry.contains(slot.lens_id) && slot.is_compute_on_recall_sidecar()
            {
                // Declared compute-on-recall: absence here is the contract, not a
                // failure, and is exempt from degradation (#1947).
                absent(AbsentReason::NotApplicable)
            } else if !state.registry.contains(slot.lens_id)
                && slot.modality == Modality::Structured
            {
                return Err(CalyxError::lens_unreachable(format!(
                    "active structured slot {} references unregistered lens {}",
                    slot.slot_id.get(),
                    slot.lens_id
                ))
                .into());
            } else if !state.registry.contains(slot.lens_id) {
                absent(AbsentReason::LensUnavailable)
            } else if state.registry.requires_structured_record(slot.lens_id)? {
                measured_vector(&measured, slot, i)?
            } else if !state.registry.accepts_input(slot.lens_id, input)? {
                absent(AbsentReason::NotApplicable)
            } else {
                measured_vector(&measured, slot, i)?
            };
            degraded |= slot
                .counts_toward_degraded_for(input.modality, input.structured().is_some())
                && vector.is_absent();
            slots.insert(slot.slot_id, vector);
        }
        let cx_id = {
            let identity = input.identity_bytes()?;
            vault.cx_id_for_input(identity.as_ref(), state.panel.version)
        };
        let mut scalars = BTreeMap::new();
        let mut metadata = BTreeMap::new();
        input.persist_structured(&mut scalars, &mut metadata)?;
        out.push(Constellation {
            cx_id,
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
        });
    }
    ingest_runtime_log(format_args!(
        "phase=measure_microbatch_ok modality={:?} batch_size={} gpu_lenses={} cpu_lenses={} elapsed_ms={}",
        batch_modality,
        inputs.len(),
        gpu_lenses.len(),
        cpu_lenses.len(),
        started.elapsed().as_millis()
    ));
    Ok(out)
}

fn measured_vector(
    measured: &std::collections::BTreeMap<LensId, Vec<SlotVector>>,
    slot: &calyx_core::Slot,
    input_index: usize,
) -> CliResult<SlotVector> {
    match measured.get(&slot.lens_id) {
        Some(vectors) if input_index < vectors.len() => Ok(vectors[input_index].clone()),
        Some(vectors) => Err(CalyxError::lens_dim_mismatch(format!(
            "lens {} produced {} vectors, missing input index {input_index}",
            slot.lens_id,
            vectors.len()
        ))
        .into()),
        None => Err(CalyxError::lens_unreachable(format!(
            "active registered slot {} lens {} was not measured",
            slot.slot_id.get(),
            slot.lens_id
        ))
        .into()),
    }
}

mod resident_batch;
