use calyx_aster::dedup::{EpochSecs, OccurrenceId};
use calyx_aster::recurrence;
use calyx_aster::vault::AsterVault;
use calyx_core::{Clock, CxId, Result};

use super::{PeriodicRecallHit, PeriodicRecallQuery, PeriodicRecallReadback, RecurrenceRead};

pub struct SeriesStore<'a, C> {
    vault: &'a AsterVault<C>,
    retention: recurrence::RetentionPolicy,
}

impl<'a, C> SeriesStore<'a, C>
where
    C: Clock,
{
    pub fn new(vault: &'a AsterVault<C>) -> Self {
        Self {
            vault,
            retention: recurrence::RetentionPolicy::default(),
        }
    }

    pub fn with_retention(
        vault: &'a AsterVault<C>,
        retention: recurrence::RetentionPolicy,
    ) -> Result<Self> {
        retention.validate()?;
        Ok(Self { vault, retention })
    }

    pub(crate) fn vault(&self) -> &'a AsterVault<C> {
        self.vault
    }

    pub fn append_occurrence(
        &self,
        cx_id: CxId,
        t_k: EpochSecs,
        context: recurrence::OccurrenceContext,
    ) -> Result<OccurrenceId> {
        self.append_occurrence_observed_at(cx_id, t_k, context, t_k)
    }

    pub fn append_occurrence_observed_at(
        &self,
        cx_id: CxId,
        t_k: EpochSecs,
        context: recurrence::OccurrenceContext,
        observed_at: EpochSecs,
    ) -> Result<OccurrenceId> {
        recurrence::append_occurrence(self.vault, cx_id, t_k, context, observed_at, self.retention)
    }

    pub fn read_series(&self, cx_id: CxId) -> Result<recurrence::RecurrenceSeries> {
        recurrence::read_series(self.vault, cx_id)
    }

    pub fn recurrence_series(&self, cx_id: CxId) -> Result<RecurrenceRead> {
        super::recurrence_series(self.vault, cx_id)
    }

    pub fn recurrence_series_with_tz_offset(
        &self,
        cx_id: CxId,
        tz_offset_secs: i32,
    ) -> Result<RecurrenceRead> {
        super::recurrence_series_with_tz_offset(self.vault, cx_id, tz_offset_secs)
    }

    pub fn occurrence_count(&self, cx_id: CxId) -> Result<u64> {
        recurrence::occurrence_count(self.vault, cx_id)
    }

    pub fn periodic_recall(&self, query: PeriodicRecallQuery) -> Result<Vec<PeriodicRecallHit>> {
        super::periodic_recall(self.vault, query)
    }

    pub fn periodic_recall_readback(
        &self,
        query: PeriodicRecallQuery,
    ) -> Result<PeriodicRecallReadback> {
        super::periodic_recall_readback(self.vault, query)
    }
}
