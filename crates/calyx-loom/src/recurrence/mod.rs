//! Bounded recurrence-series storage over Aster recurrence CF rows.

mod series_store;
pub mod signature;

pub use calyx_aster::recurrence::{
    FREQUENCY_SCALAR, MAX_CONTEXT_BYTES, Occurrence, OccurrenceContext, RecurrenceSeries,
    RetentionPolicy, RollupSummary, StoredRecurrenceRow, decode_recurrence_row,
    encode_recurrence_row, recurrence_summary_key,
};
pub use series_store::SeriesStore;
pub use signature::{
    CALYX_RECURRENCE_SLOT_MISSING, SignatureResult, detect_recurrence_signature,
    temporal_slot_ids_for_panel,
};

#[cfg(test)]
mod tests;
