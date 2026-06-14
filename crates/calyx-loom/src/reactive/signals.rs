//! Real [`ReactiveSignals`] sources backing reactive conditions.

use calyx_aster::vault::AsterVault;
use calyx_core::{Clock, CxId, Result, SlotId};

use super::{NoveltyVerdict, ReactiveSignals};
use crate::error::{CALYX_REACTIVE_SIGNAL_UNAVAILABLE, loom_error};
use crate::recurrence::SeriesStore;

/// A [`ReactiveSignals`] source backed by the durable recurrence store. It
/// answers [`super::TriggerCondition::EventRecurs`] from the real, on-disk
/// occurrence count via [`SeriesStore::occurrence_count`].
///
/// Novelty and drift are outside this source's domain — it owns no Ward profile
/// and no agreement graph — so those methods **fail closed** with
/// [`CALYX_REACTIVE_SIGNAL_UNAVAILABLE`] rather than silently report "no match".
/// Compose a Ward-backed and an agreement-graph-backed source to cover
/// `NewRegion` and `DriftDetected`.
pub struct RecurrenceSignals<'a, C: Clock> {
    store: SeriesStore<'a, C>,
}

impl<'a, C: Clock> RecurrenceSignals<'a, C> {
    /// Wraps `vault`'s recurrence store as a reactive signal source.
    pub fn new(vault: &'a AsterVault<C>) -> Self {
        Self {
            store: SeriesStore::new(vault),
        }
    }
}

impl<C: Clock> ReactiveSignals for RecurrenceSignals<'_, C> {
    fn novelty(&self, _cx_id: CxId, _tau_override: Option<f32>) -> Result<NoveltyVerdict> {
        Err(loom_error(
            CALYX_REACTIVE_SIGNAL_UNAVAILABLE,
            "recurrence signal source cannot evaluate a NewRegion novelty verdict",
        ))
    }

    fn occurrence_count(&self, series: CxId) -> Result<u64> {
        self.store.occurrence_count(series)
    }

    fn slot_drift(&self, _slot: SlotId) -> Result<f32> {
        Err(loom_error(
            CALYX_REACTIVE_SIGNAL_UNAVAILABLE,
            "recurrence signal source cannot evaluate a DriftDetected delta",
        ))
    }
}
