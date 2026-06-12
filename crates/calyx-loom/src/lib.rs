//! Loom DDA cross-term and agreement-graph engine.

pub mod abundance;
pub mod agreement_graph;
pub mod blind_spot;
pub mod cross_term;
pub mod error;
pub mod lru_cache;
pub mod materialization;
pub mod recurrence;

pub use abundance::{
    AbundanceReport, CeilingEstimate, NeffEstimate, cross_term_upper_bound, dda_signal_yield,
    meaning_compression_yield,
};
pub use agreement_graph::{AgreementEdge, LoomStore};
pub use blind_spot::{BlindSpotAlert, Severity, detect_blind_spot};
pub use cross_term::{
    CrossTermKey, CrossTermKind, CrossTermValue, SignalProvenanceTag, agreement_batch_cpu,
    agreement_batch_gpu, agreement_scalar, agreement_weight, concat_vec, delta_vec,
    interaction_vec,
};
pub use error::{
    CALYX_LOOM_DIM_MISMATCH, CALYX_LOOM_FORGE_UNAVAILABLE, CALYX_LOOM_NON_FINITE_VECTOR,
    CALYX_LOOM_SERIES_READ_ERROR, CALYX_LOOM_SLOT_MISSING, CALYX_LOOM_TEMPORAL_XTERM_CORRUPT,
    CALYX_LOOM_ZERO_NORM_VECTOR, CALYX_RECURRENCE_CONTEXT_TOO_LARGE,
    CALYX_RECURRENCE_INVALID_RETENTION, loom_error,
};
pub use lru_cache::LruCache;
pub use materialization::{
    MaterializationAction, MaterializationPlan, PairGainGate, StaticPairGainGate, plan_cross_terms,
    plan_cross_terms_checked,
};
pub use recurrence::{
    LeadLagResult, Occurrence, OccurrenceContext, PeriodicFit, PeriodicRecallHit,
    PeriodicRecallQuery, PeriodicRecallReadback, PeriodicRecallStats, RecurrenceRead,
    RecurrenceReadStats, RecurrenceSeries, RecurrenceSeriesReadback, RetentionPolicy,
    RollupSummary, SeriesStore, SignatureResult, StoredRecurrenceRow, co_occurrence_pairs,
    decode_lead_lag_result, decode_recurrence_row, detect_recurrence_signature,
    encode_lead_lag_result, encode_recurrence_row, lead_lag_secs, periodic_fit, periodic_recall,
    periodic_recall_readback, recurrence_series, recurrence_summary_key, temporal_cross_term,
    temporal_slot_ids_for_panel,
};

#[cfg(test)]
mod tests {
    #[test]
    fn crate_metadata_is_present() {
        assert_eq!(env!("CARGO_PKG_NAME"), "calyx-loom");
    }
}
