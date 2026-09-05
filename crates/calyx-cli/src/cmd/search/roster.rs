//! Search-side derivation of the vault's frozen text serving roster (#1490).
//!
//! Search must serve using exactly THIS vault's frozen panel: active GPU
//! slots are demanded from the vault's own resident service, active CPU
//! slots are measured locally in-process (the same contract ingest uses),
//! and parked/retired/unregistered slots are cleanly excluded from the
//! demand-set — excluded slots are always reported (stderr roster line and
//! `--explain` output), never silently dropped. Before #1490 search demanded
//! EVERY active slot from the resident while the resident refused CPU-placed
//! content lenses (`CALYX_PANEL_RESIDENT_CPU_LENS_REFUSED`), deadlocking any
//! panel with an active CPU content lens.

use std::collections::BTreeSet;
use std::net::SocketAddr;
use std::str::FromStr;

use calyx_core::{CalyxError, LensId, Modality, Placement, Slot, SlotId, SlotState, SlotVector};
use calyx_registry::VaultPanelState;
use serde::Serialize;

use crate::error::CliResult;
use crate::panel_commands::ResidentMeasuredInput;

/// The text-search serving roster: how each text slot of the frozen panel
/// participates in query-side measurement.
pub(super) struct SearchTextRoster<'a> {
    /// Active + registered + GPU-placed: demanded from THIS vault's resident.
    pub(super) resident_gpu: Vec<&'a Slot>,
    /// Active + registered + CPU-placed: measured locally in-process, exactly
    /// like ingest measures them. Present only via the explicit CPU-lens
    /// admission opt-in (`CALYX_PANEL_ALLOW_CPU_LENS`).
    pub(super) local_cpu: Vec<&'a Slot>,
    /// Parked slots: excluded from serving, stored vectors remain readable.
    pub(super) parked_excluded: Vec<&'a Slot>,
    /// Retired slots: excluded from serving.
    pub(super) retired_excluded: Vec<&'a Slot>,
    /// Active slots whose lens is not materialized in the registry: excluded
    /// from query measurement (reported, not silent).
    pub(super) unregistered_excluded: Vec<&'a Slot>,
}

impl<'a> SearchTextRoster<'a> {
    pub(super) fn derive(state: &'a VaultPanelState) -> Self {
        let mut roster = Self {
            resident_gpu: Vec::new(),
            local_cpu: Vec::new(),
            parked_excluded: Vec::new(),
            retired_excluded: Vec::new(),
            unregistered_excluded: Vec::new(),
        };
        for slot in &state.panel.slots {
            if slot.modality != Modality::Text {
                continue;
            }
            match slot.state {
                SlotState::Parked => roster.parked_excluded.push(slot),
                SlotState::Retired => roster.retired_excluded.push(slot),
                SlotState::Active => {
                    if !state.registry.contains(slot.lens_id) {
                        roster.unregistered_excluded.push(slot);
                    } else if slot.resource.placement == Placement::Gpu {
                        roster.resident_gpu.push(slot);
                    } else {
                        roster.local_cpu.push(slot);
                    }
                }
            }
        }
        roster
    }

    /// One always-on structured stderr line stating exactly which slots serve
    /// (and how) and which are excluded — the anti-silent-partial guarantee.
    pub(super) fn emit_runtime_line(&self) {
        eprintln!(
            "CALYX_SEARCH_SLOTS resident_gpu=[{}] local_cpu=[{}] parked_excluded=[{}] retired_excluded=[{}] unregistered_excluded=[{}]",
            slot_list(&self.resident_gpu),
            slot_list(&self.local_cpu),
            slot_list(&self.parked_excluded),
            slot_list(&self.retired_excluded),
            slot_list(&self.unregistered_excluded)
        );
    }

    pub(super) fn to_out(&self) -> SlotRosterOut {
        SlotRosterOut {
            resident_gpu: slot_refs(&self.resident_gpu),
            local_cpu: slot_refs(&self.local_cpu),
            parked_excluded: slot_refs(&self.parked_excluded),
            retired_excluded: slot_refs(&self.retired_excluded),
            unregistered_excluded: slot_refs(&self.unregistered_excluded),
        }
    }
}

/// Roster block rendered into the `--explain` search response so operators
/// see which slots contributed and which were parked/excluded.
#[derive(Debug, Serialize)]
pub(super) struct SlotRosterOut {
    pub resident_gpu: Vec<SlotRefOut>,
    pub local_cpu: Vec<SlotRefOut>,
    pub parked_excluded: Vec<SlotRefOut>,
    pub retired_excluded: Vec<SlotRefOut>,
    pub unregistered_excluded: Vec<SlotRefOut>,
}

#[derive(Debug, Serialize)]
pub(super) struct SlotRefOut {
    pub slot: u16,
    pub key: String,
    pub lens_id: String,
    pub placement: String,
}

fn slot_refs(slots: &[&Slot]) -> Vec<SlotRefOut> {
    slots
        .iter()
        .map(|slot| SlotRefOut {
            slot: slot.slot_id.get(),
            key: slot.slot_key.key().to_string(),
            lens_id: slot.lens_id.to_string(),
            placement: format!("{:?}", slot.resource.placement),
        })
        .collect()
}

fn slot_list(slots: &[&Slot]) -> String {
    slots
        .iter()
        .map(|slot| format!("{}:{}", slot.slot_id.get(), slot.slot_key.key()))
        .collect::<Vec<_>>()
        .join(",")
}

/// Validate one resident measure-batch row against the demanded GPU slots and
/// extract contract-verified query vectors. The demand-set is EXACTLY the
/// active GPU roster — parked/CPU slots are never demanded from the resident.
pub(super) fn query_vectors_from_resident_row(
    state: &VaultPanelState,
    demanded: &[&Slot],
    row: &ResidentMeasuredInput,
    addr: SocketAddr,
) -> CliResult<Vec<(SlotId, SlotVector)>> {
    let mut out = Vec::with_capacity(demanded.len());
    for slot in demanded {
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
                    "resident service {addr} did not return active GPU text slot {} lens {}",
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
        if !vector.is_indexable() {
            return Err(CalyxError::stale_derived(format!(
                "resident service {addr} returned non-indexable query vector for slot {} lens {}",
                slot.slot_id.get(),
                slot.lens_id
            ))
            .into());
        }
        verify_against_snapshot(state, slot, &vector, "resident")?;
        out.push((slot.slot_id, vector));
    }
    Ok(out)
}

/// Measure the active CPU-placed slots locally in-process — the same frozen
/// registry contract path ingest uses for CPU lenses. Every demanded CPU slot
/// must yield a contract-verified indexable vector (no silent partials).
pub(super) fn measure_local_cpu_query_vectors(
    state: &VaultPanelState,
    demanded: &[&Slot],
    query: &str,
) -> CliResult<Vec<(SlotId, SlotVector)>> {
    if demanded.is_empty() {
        return Ok(Vec::new());
    }
    for slot in demanded {
        eprintln!(
            "CALYX_SEARCH_RUNTIME phase=search_local_cpu_measure slot={} key={} lens={} placement={:?}",
            slot.slot_id.get(),
            slot.slot_key.key(),
            slot.lens_id,
            slot.resource.placement
        );
    }
    let allowed = demanded
        .iter()
        .map(|slot| slot.slot_id)
        .collect::<BTreeSet<_>>();
    let measured = calyx_search::measure_query_vectors_with_slots(state, query, Some(&allowed))?;
    let mut out = Vec::with_capacity(demanded.len());
    for slot in demanded {
        let vector = measured
            .iter()
            .find(|(slot_id, _)| *slot_id == slot.slot_id)
            .map(|(_, vector)| vector.clone())
            .ok_or_else(|| {
                CalyxError::stale_derived(format!(
                    "local in-process measurement produced no indexable query vector for active CPU text slot {} lens {}",
                    slot.slot_id.get(),
                    slot.lens_id
                ))
            })?;
        verify_against_snapshot(state, slot, &vector, "local CPU")?;
        out.push((slot.slot_id, vector));
    }
    Ok(out)
}

/// Fail-closed project binding: the resident must serve THIS vault's frozen
/// panel (its `template_source` is the canonical vault path). A resident
/// warmed for another vault/template on the same box is refused loudly —
/// search never silently binds to another project's warm lanes.
pub(super) fn require_resident_template_binding(
    actual: &str,
    expected: &str,
    addr: SocketAddr,
) -> CliResult {
    if actual == expected {
        return Ok(());
    }
    Err(CalyxError {
        code: "CALYX_SEARCH_RESIDENT_MISMATCH",
        message: format!(
            "resident service {addr} served template_source {actual}, expected {expected}"
        ),
        remediation: "restart the resident service with `calyx panel resident serve --vault <this-vault-path>` and retry search",
    }
    .into())
}

fn verify_against_snapshot(
    state: &VaultPanelState,
    slot: &Slot,
    vector: &SlotVector,
    origin: &str,
) -> CliResult {
    let snapshot = registry_snapshot_for_lens(state, slot.lens_id).ok_or_else(|| {
        CalyxError::lens_unreachable(format!(
            "{origin} search measurement requires persisted registry snapshot contract for lens {}",
            slot.lens_id
        ))
    })?;
    snapshot.contract.verify_vector(slot.lens_id, vector)?;
    Ok(())
}

fn registry_snapshot_for_lens(
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

#[cfg(test)]
mod tests;
