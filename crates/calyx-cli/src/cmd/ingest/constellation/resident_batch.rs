use std::net::SocketAddr;

use super::*;

pub(super) fn measure_gpu_lenses_via_resident_service(
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
