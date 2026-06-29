use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::str::FromStr;
use std::time::Instant;

use calyx_aster::vault::AsterVault;
use calyx_core::{
    AbsentReason, CalyxError, CalyxErrorCode, Constellation, CxFlags, Input, InputRef, LedgerRef,
    LensId, Modality, Placement, SlotId, SlotState, SlotVector,
};
use calyx_registry::VaultPanelState;
pub(crate) use calyx_registry::measure::{absent, input_hash};
use rayon::prelude::*;

use super::command::ingest_runtime_log;
use super::worker::measure_lens_in_worker;
use crate::error::CliResult;
use crate::lens_commands::support::runtime_name;
use crate::panel_commands::measure_resident_batch_at;

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
    let mut declared = 0usize;
    let mut absent: Vec<String> = Vec::new();
    for slot in &state.panel.slots {
        if !slot.counts_toward_degraded(cx.modality) {
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

#[derive(Clone, Copy)]
struct ApplicableLens {
    lens_id: LensId,
    slot_id: SlotId,
    placement: Placement,
}

fn persisted_snapshot_for_lens(
    state: &VaultPanelState,
    lens_id: LensId,
) -> Option<&calyx_registry::RegistryLensSnapshot> {
    state
        .registry_snapshot
        .as_ref()?
        .lenses
        .iter()
        .find(|snapshot| snapshot.lens_id == lens_id)
}

fn measure_persisted_lens_in_worker(
    snapshot: &calyx_registry::RegistryLensSnapshot,
    inputs: &[Input],
    runtime_batch_limit: Option<usize>,
) -> calyx_core::Result<Vec<SlotVector>> {
    let vectors = measure_lens_in_worker(snapshot, inputs, runtime_batch_limit)?;
    if vectors.len() != inputs.len() {
        return Err(CalyxError::lens_dim_mismatch(format!(
            "ingest lens worker for lens {} returned {} vectors for {} inputs",
            snapshot.lens_id,
            vectors.len(),
            inputs.len()
        )));
    }
    for vector in &vectors {
        snapshot.contract.verify_vector(snapshot.lens_id, vector)?;
    }
    Ok(vectors)
}

fn measure_registry_lens_batch_with_limit(
    state: &VaultPanelState,
    lens_id: LensId,
    inputs: &[Input],
    runtime_batch_limit: Option<usize>,
) -> calyx_core::Result<Vec<SlotVector>> {
    let Some(limit) = runtime_batch_limit else {
        return state.registry.measure_batch(lens_id, inputs);
    };
    if limit == 0 {
        return Err(CalyxError::lens_unreachable(
            "runtime batch limit must be > 0 when supplied",
        ));
    }
    let mut out = Vec::with_capacity(inputs.len());
    for chunk in inputs.chunks(limit) {
        out.extend(state.registry.measure_batch(lens_id, chunk)?);
    }
    Ok(out)
}

fn measure_applicable_lens_batch(
    state: &VaultPanelState,
    lens: ApplicableLens,
    modality: Modality,
    inputs: &[Input],
    runtime_batch_limit: Option<usize>,
) -> calyx_core::Result<Vec<SlotVector>> {
    let started = Instant::now();
    let spec = state.registry.lens_spec(lens.lens_id);
    let spec_name = spec
        .map(|spec| spec.name.as_str())
        .unwrap_or("missing_registry_snapshot");
    let runtime = spec
        .map(|spec| runtime_name(&spec.runtime))
        .unwrap_or("unregistered");
    ingest_runtime_log(format_args!(
        "phase=measure_lens_start lens_id={} slot={} name={} runtime={} modality={:?} placement={:?} batch_size={} runtime_batch_limit={:?}",
        lens.lens_id,
        lens.slot_id.get(),
        spec_name,
        runtime,
        modality,
        lens.placement,
        inputs.len(),
        runtime_batch_limit
    ));
    let result = if lens.placement == Placement::Gpu {
        if let Some(snapshot) = persisted_snapshot_for_lens(state, lens.lens_id) {
            ingest_runtime_log(format_args!(
                "phase=measure_lens_worker_start lens_id={} slot={} name={} inputs={} runtime_batch_limit={:?}",
                lens.lens_id,
                lens.slot_id.get(),
                spec_name,
                inputs.len(),
                runtime_batch_limit
            ));
            let result = measure_persisted_lens_in_worker(snapshot, inputs, runtime_batch_limit);
            if result.is_ok() {
                ingest_runtime_log(format_args!(
                    "phase=measure_lens_worker_ok lens_id={} slot={} name={}",
                    lens.lens_id,
                    lens.slot_id.get(),
                    spec_name
                ));
            }
            result
        } else {
            measure_registry_lens_batch_with_limit(state, lens.lens_id, inputs, runtime_batch_limit)
        }
    } else {
        measure_registry_lens_batch_with_limit(state, lens.lens_id, inputs, runtime_batch_limit)
    };
    match &result {
        Ok(vectors) => ingest_runtime_log(format_args!(
            "phase=measure_lens_ok lens_id={} slot={} name={} vectors={} elapsed_ms={}",
            lens.lens_id,
            lens.slot_id.get(),
            spec_name,
            vectors.len(),
            started.elapsed().as_millis()
        )),
        Err(error) => ingest_runtime_log(format_args!(
            "phase=measure_lens_err lens_id={} slot={} name={} code={} message={} elapsed_ms={}",
            lens.lens_id,
            lens.slot_id.get(),
            spec_name,
            error.code,
            error.message,
            started.elapsed().as_millis()
        )),
    }
    result
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
    measure_constellation_microbatch_with_runtime_limit(vault, state, inputs, now, None, None)
}

pub(crate) fn measure_constellation_microbatch_with_runtime_limit(
    vault: &AsterVault,
    state: &VaultPanelState,
    inputs: &[Input],
    now: u64,
    runtime_batch_limit: Option<usize>,
    resident_addr: Option<SocketAddr>,
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
        if slot.state == SlotState::Active
            && slot.modality == batch_modality
            && state.registry.contains(slot.lens_id)
        {
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
    }
    ingest_runtime_log(format_args!(
        "phase=measure_microbatch_start modality={:?} batch_size={} gpu_lenses={} cpu_lenses={} runtime_batch_limit={:?} resident_addr={:?}",
        batch_modality,
        inputs.len(),
        gpu_lenses.len(),
        cpu_lenses.len(),
        runtime_batch_limit,
        resident_addr
    ));
    let measure_one = |lens: ApplicableLens| {
        measure_applicable_lens_batch(state, lens, batch_modality, inputs, runtime_batch_limit)
            .map(|vectors| (lens.lens_id, vectors))
    };
    let (gpu_result, cpu_result) = rayon::join(
        || match resident_addr {
            Some(addr) => measure_gpu_lenses_via_resident_service(
                state,
                &gpu_lenses,
                batch_modality,
                inputs,
                runtime_batch_limit,
                addr,
            ),
            None => gpu_lenses
                .iter()
                .map(|&lens| measure_one(lens))
                .collect::<std::result::Result<Vec<_>, _>>(),
        },
        || {
            cpu_lenses
                .par_iter()
                .map(|&lens| measure_one(lens))
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
                    Some(vectors) => {
                        return Err(CalyxError::lens_dim_mismatch(format!(
                            "lens {} produced {} vectors, missing input index {i}",
                            slot.lens_id,
                            vectors.len()
                        ))
                        .into());
                    }
                    None => {
                        return Err(CalyxError::lens_unreachable(format!(
                            "active registered slot {} lens {} was not measured",
                            slot.slot_id.get(),
                            slot.lens_id
                        ))
                        .into());
                    }
                }
            };
            degraded |= slot.counts_toward_degraded(input.modality) && vector.is_absent();
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

fn measure_gpu_lenses_via_resident_service(
    state: &VaultPanelState,
    gpu_lenses: &[ApplicableLens],
    modality: Modality,
    inputs: &[Input],
    runtime_batch_limit: Option<usize>,
    addr: SocketAddr,
) -> calyx_core::Result<Vec<(LensId, Vec<SlotVector>)>> {
    if gpu_lenses.is_empty() {
        return Ok(Vec::new());
    }
    let started = Instant::now();
    ingest_runtime_log(format_args!(
        "phase=measure_resident_service_start addr={} modality={:?} inputs={} gpu_lenses={} runtime_batch_limit={:?}",
        addr,
        modality,
        inputs.len(),
        gpu_lenses.len(),
        runtime_batch_limit
    ));
    let resident = measure_resident_batch_at(addr, modality, inputs, runtime_batch_limit)
        .map_err(|error| CalyxError {
            code: "CALYX_PANEL_RESIDENT_UNAVAILABLE",
            message: format!(
                "resident service measurement failed addr={addr} code={} message={}",
                error.code(),
                error.message()
            ),
            remediation: "start `calyx panel resident serve` for the matching GPU panel on the requested loopback address, then retry ingest",
        })?;
    let request_bytes = resident.request_bytes;
    let response_bytes = resident.response_bytes;
    let response = resident.response;
    if !response.ready {
        return Err(CalyxError::lens_unreachable(format!(
            "resident service {addr} returned ready=false for measure_batch"
        )));
    }
    if response.modality != modality || response.input_count != inputs.len() {
        return Err(CalyxError::lens_dim_mismatch(format!(
            "resident service {addr} response mismatch: modality {:?} input_count {}, expected {:?} {}",
            response.modality,
            response.input_count,
            modality,
            inputs.len()
        )));
    }
    if response.rows.len() != inputs.len() {
        return Err(CalyxError::lens_dim_mismatch(format!(
            "resident service {addr} returned {} rows for {} inputs",
            response.rows.len(),
            inputs.len()
        )));
    }
    let required = gpu_lenses
        .iter()
        .map(|lens| lens.lens_id)
        .collect::<Vec<_>>();
    for lens_id in &required {
        if persisted_snapshot_for_lens(state, *lens_id).is_none() {
            return Err(CalyxError::lens_unreachable(format!(
                "resident ingest requires persisted registry snapshot contract for GPU lens {lens_id}"
            )));
        }
    }
    let mut by_lens = required
        .iter()
        .map(|lens_id| (*lens_id, Vec::with_capacity(inputs.len())))
        .collect::<BTreeMap<_, _>>();
    for (expected_index, row) in response.rows.iter().enumerate() {
        if row.input_index != expected_index {
            return Err(CalyxError::lens_dim_mismatch(format!(
                "resident service {addr} returned row index {}, expected {}",
                row.input_index, expected_index
            )));
        }
        for lens_id in &required {
            let slot = row
                .slots
                .iter()
                .find(|slot| {
                    slot.measured
                        && LensId::from_str(&slot.lens_id)
                            .map(|returned| returned == *lens_id)
                            .unwrap_or(false)
                })
                .ok_or_else(|| {
                    CalyxError::lens_unreachable(format!(
                        "resident service {addr} did not return required GPU lens {lens_id} for input {expected_index}"
                    ))
                })?;
            if slot.modality != modality || slot.placement != Placement::Gpu {
                return Err(CalyxError::lens_unreachable(format!(
                    "resident service {addr} returned lens {lens_id} with modality {:?} placement {:?}, expected {:?}/Gpu",
                    slot.modality, slot.placement, modality
                )));
            }
            let vector = slot.vector.clone().ok_or_else(|| {
                CalyxError::lens_unreachable(format!(
                    "resident service {addr} measured lens {lens_id} input {expected_index} without a vector"
                ))
            })?;
            persisted_snapshot_for_lens(state, *lens_id)
                .expect("checked above")
                .contract
                .verify_vector(*lens_id, &vector)?;
            by_lens
                .get_mut(lens_id)
                .expect("initialized required lens")
                .push(vector);
        }
    }
    let out = by_lens.into_iter().collect::<Vec<_>>();
    ingest_runtime_log(format_args!(
        "phase=measure_resident_service_ok addr={} process_id={} template_source={} inputs={} gpu_lenses={} elapsed_ms={} resident_elapsed_ms={} runtime_batch_limit={:?} protocol=binary request_bytes={} response_bytes={}",
        addr,
        response.process_id,
        response.template_source,
        inputs.len(),
        gpu_lenses.len(),
        started.elapsed().as_millis(),
        response.elapsed_ms,
        response.runtime_batch_limit,
        request_bytes,
        response_bytes
    ));
    Ok(out)
}
