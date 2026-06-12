//! Bounded recurrence-series storage over Aster recurrence CF rows.

pub mod cross_terms;
mod periodic;
mod series_store;
pub mod signature;

pub use calyx_aster::recurrence::{
    FREQUENCY_SCALAR, MAX_CONTEXT_BYTES, Occurrence, OccurrenceContext, RecurrenceReadStats,
    RecurrenceSeries, RecurrenceSeriesReadback, RetentionPolicy, RollupSummary,
    StoredRecurrenceRow, decode_recurrence_row, encode_recurrence_row, recurrence_summary_key,
};
pub use cross_terms::{
    LeadLagResult, co_occurrence_pairs, decode_lead_lag_result, encode_lead_lag_result,
    lead_lag_secs, temporal_cross_term,
};
pub use periodic::{
    PeriodicFit, PeriodicRecallHit, PeriodicRecallQuery, PeriodicRecallReadback,
    PeriodicRecallStats, PeriodicTimeBucket, RecurrenceRead, periodic_fit,
    periodic_fit_with_tz_offset, periodic_recall, periodic_recall_readback, periodic_time_bucket,
    recurrence_series, recurrence_series_with_tz_offset,
};
pub use series_store::SeriesStore;
pub use signature::{
    CALYX_RECURRENCE_SLOT_MISSING, SignatureResult, detect_recurrence_signature,
    temporal_slot_ids_for_panel,
};

#[cfg(test)]
mod tests;
