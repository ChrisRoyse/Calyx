use calyx_core::{CalyxError, Panel, Slot, SlotState};
use calyx_registry::Registry;
use serde::Serialize;

use crate::error::{CliError, CliResult};

/// Canonical content-slot predicate, shared by create-vault and warm/probe
/// (issue #1924) so the two counters cannot drift again. A "content slot" is a
/// dedup-participating, non-retrieval-only slot; retrieval-only / temporal
/// sidecars (e.g. `E2_recency`) are declared but never count as content lenses.
pub(crate) fn is_content_slot(slot: &Slot) -> bool {
    !slot.retrieval_only && !slot.excluded_from_dedup
}

/// The canonical "materialized content lens" predicate: a content slot that is
/// both `Active` and resident in the registry — i.e. a lens that will actually
/// measure at ingest. create-vault previously counted every declared content
/// slot as materialized (issue #1924), conflating parked slots with live ones.
pub(crate) fn is_active_materialized_content_lens(slot: &Slot, registry: &Registry) -> bool {
    is_content_slot(slot) && slot.state == SlotState::Active && registry.contains(slot.lens_id)
}

/// A content slot that is not active — declared by the template but parked /
/// unmaterialized (no local runtime, or awaiting external registration).
fn is_inactive_content_slot(slot: &Slot) -> bool {
    is_content_slot(slot) && slot.state != SlotState::Active
}

/// Honest content-slot accounting for a prepared vault panel.
///
/// `declared_content_slots` counts every content slot the template declares;
/// `active_materialized_content_lenses` counts only the ones that are Active and
/// registry-resident; `inactive_unmaterialized_slots` names the remaining
/// declared content slots that are parked.
#[derive(Clone, Debug, Serialize)]
pub(crate) struct ContentSlotReport {
    pub(crate) declared_content_slots: usize,
    pub(crate) active_materialized_content_lenses: usize,
    pub(crate) inactive_unmaterialized_slots: Vec<String>,
}

impl ContentSlotReport {
    /// Derive the report from the actual persisted panel + registry (the source
    /// of truth), and fail closed on the invariant violation
    /// `declared == active_materialized + parked`. The only way that can fail is
    /// an Active content slot with no registry entry — a corrupt panel that must
    /// never be persisted as a vault.
    pub(crate) fn derive(panel: &Panel, registry: &Registry) -> CliResult<Self> {
        let declared_content_slots = panel
            .slots
            .iter()
            .filter(|slot| is_content_slot(slot))
            .count();
        let active_materialized_content_lenses = panel
            .slots
            .iter()
            .filter(|slot| is_active_materialized_content_lens(slot, registry))
            .count();
        let inactive_unmaterialized_slots = panel
            .slots
            .iter()
            .filter(|slot| is_inactive_content_slot(slot))
            .map(|slot| slot.slot_key.key().to_string())
            .collect::<Vec<_>>();

        let parked = inactive_unmaterialized_slots.len();
        if declared_content_slots != active_materialized_content_lenses + parked {
            return Err(CliError::from(CalyxError::aster_corrupt_shard(format!(
                "content-slot invariant violated for prepared panel: declared={declared_content_slots} != active_materialized={active_materialized_content_lenses} + parked={parked}; an active content slot has no registry entry"
            ))));
        }

        Ok(Self {
            declared_content_slots,
            active_materialized_content_lenses,
            inactive_unmaterialized_slots,
        })
    }
}
