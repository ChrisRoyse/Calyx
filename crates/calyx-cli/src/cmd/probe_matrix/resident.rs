use std::collections::BTreeSet;
use std::net::SocketAddr;
use std::path::Path;
use std::str::FromStr;
use std::time::Instant;

use calyx_core::{CalyxError, Input, LensId, Modality, Placement, SlotId, SlotState, SlotVector};
use calyx_registry::VaultPanelState;

use crate::error::CliResult;
use crate::panel_commands::measure_resident_batch_at;
use crate::path_identity::vault_template_source;

pub(super) fn require_resident_for_gpu_text_slots(
    state: &VaultPanelState,
    selected_slots: &[SlotId],
) -> CliResult<()> {
    let gpu_slots = state
        .panel
        .slots
        .iter()
        .filter(|slot| selected_slots.contains(&slot.slot_id))
        .filter(|slot| slot.state == SlotState::Active && slot.modality == Modality::Text)
        .filter(|slot| slot.resource.placement == Placement::Gpu)
        .map(|slot| {
            format!(
                "slot={} key={} lens={} placement={:?}",
                slot.slot_id.get(),
                slot.slot_key.key(),
                slot.lens_id,
                slot.resource.placement
            )
        })
        .collect::<Vec<_>>();
    if gpu_slots.is_empty() {
        return Ok(());
    }
    Err(CalyxError {
        code: "CALYX_PROBE_MATRIX_RESIDENT_REQUIRED",
        message: format!(
            "probe-matrix refuses cold local query measurement for {} selected GPU text lens(es): {}",
            gpu_slots.len(),
            gpu_slots.join(", ")
        ),
        remediation:
            "start `calyx panel resident serve --vault <vault-path>` and rerun probe-matrix with --resident-addr <127.0.0.1:port>",
    }
    .into())
}

pub(super) fn measure_query_vectors_via_resident(
    state: &VaultPanelState,
    vault_dir: &Path,
    query: &str,
    allowed_slots: &BTreeSet<SlotId>,
    addr: SocketAddr,
) -> CliResult<Vec<(SlotId, SlotVector)>> {
    let started = Instant::now();
    let input = Input::new(Modality::Text, query.as_bytes().to_vec());
    eprintln!(
        "probe-matrix: resident measurement start addr={} vault={} inputs=1 selected_slots={}",
        addr,
        vault_dir.display(),
        allowed_slots.len()
    );
    let resident = measure_resident_batch_at(addr, Modality::Text, &[input], None).map_err(
        |error| CalyxError {
            code: "CALYX_PANEL_RESIDENT_UNAVAILABLE",
            message: format!(
                "resident probe-matrix measurement failed addr={addr} code={} message={}",
                error.code(),
                error.message()
            ),
            remediation:
                "start `calyx panel resident serve --vault <vault-path>` for the matching active vault, then retry probe-matrix with --resident-addr",
        },
    )?;
    let request_bytes = resident.request_bytes;
    let response_bytes = resident.response_bytes;
    let response = resident.response;
    if !response.ready {
        return Err(CalyxError::lens_unreachable(format!(
            "resident service {addr} returned ready=false for probe-matrix measurement"
        ))
        .into());
    }
    let expected_template_source = vault_template_source(vault_dir)?;
    if response.template_source != expected_template_source {
        return Err(CalyxError {
            code: "CALYX_PROBE_MATRIX_RESIDENT_MISMATCH",
            message: format!(
                "resident service {addr} served template_source {}, expected {}",
                response.template_source, expected_template_source
            ),
            remediation: "restart the resident service with `calyx panel resident serve --vault <this-vault-path>` and retry probe-matrix",
        }
        .into());
    }
    if response.modality != Modality::Text || response.input_count != 1 || response.rows.len() != 1
    {
        return Err(CalyxError::lens_dim_mismatch(format!(
            "resident probe-matrix response mismatch: modality {:?} input_count {} rows {}, expected Text/1/1",
            response.modality,
            response.input_count,
            response.rows.len()
        ))
        .into());
    }
    let row = &response.rows[0];
    if row.input_index != 0 {
        return Err(CalyxError::lens_dim_mismatch(format!(
            "resident probe-matrix response row index {}, expected 0",
            row.input_index
        ))
        .into());
    }

    let mut out = Vec::new();
    for slot_id in allowed_slots {
        let slot = state
            .panel
            .slots
            .iter()
            .find(|slot| slot.slot_id == *slot_id)
            .ok_or_else(|| {
                CalyxError::lens_unreachable(format!("selected slot {slot_id} missing from panel"))
            })?;
        let returned = row
            .slots
            .iter()
            .find(|returned| {
                returned.measured
                    && returned.slot == slot.slot_id.get()
                    && LensId::from_str(&returned.lens_id)
                        .map(|lens_id| lens_id == slot.lens_id)
                        .unwrap_or(false)
            })
            .ok_or_else(|| {
                CalyxError::lens_unreachable(format!(
                    "resident service {addr} did not return selected text slot {} lens {}",
                    slot.slot_id.get(),
                    slot.lens_id
                ))
            })?;
        if returned.modality != Modality::Text || returned.placement != slot.resource.placement {
            return Err(CalyxError::lens_unreachable(format!(
                "resident service {addr} returned slot {} lens {} with modality {:?} placement {:?}, expected Text/{:?}",
                slot.slot_id.get(),
                slot.lens_id,
                returned.modality,
                returned.placement,
                slot.resource.placement
            ))
            .into());
        }
        let vector = returned.vector.clone().ok_or_else(|| {
            CalyxError::lens_unreachable(format!(
                "resident service {addr} measured slot {} lens {} without a vector",
                slot.slot_id.get(),
                slot.lens_id
            ))
        })?;
        if !indexable(&vector) {
            return Err(CalyxError::stale_derived(format!(
                "resident service {addr} returned non-indexable query vector for slot {} lens {}",
                slot.slot_id.get(),
                slot.lens_id
            ))
            .into());
        }
        let snapshot = state
            .registry_snapshot
            .as_ref()
            .and_then(|snapshot| {
                snapshot
                    .lenses
                    .iter()
                    .find(|candidate| candidate.lens_id == slot.lens_id)
            })
            .ok_or_else(|| {
                CalyxError::lens_unreachable(format!(
                    "probe-matrix requires persisted registry snapshot contract for lens {}",
                    slot.lens_id
                ))
            })?;
        snapshot.contract.verify_vector(slot.lens_id, &vector)?;
        out.push((slot.slot_id, vector));
    }
    eprintln!(
        "probe-matrix: resident measurement ok addr={} process_id={} template_source={} inputs=1 slots={} elapsed_ms={} resident_elapsed_ms={} protocol=binary request_bytes={} response_bytes={}",
        addr,
        response.process_id,
        response.template_source,
        out.len(),
        started.elapsed().as_millis(),
        response.elapsed_ms,
        request_bytes,
        response_bytes
    );
    Ok(out)
}

fn indexable(vector: &SlotVector) -> bool {
    matches!(
        vector,
        SlotVector::Dense { .. } | SlotVector::Sparse { .. } | SlotVector::Multi { .. }
    )
}
