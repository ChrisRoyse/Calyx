//! Temporal search policy types for AP-60 post-retrieval boosting.

mod boost;
mod causal_gate;
mod recall_budget;
mod recurrence_boost;
mod search;
mod window;

pub use boost::{
    TemporalScores, TemporalTimeBucket, apply_temporal_boost, apply_temporal_boost_with_recurrence,
    fuse_temporal, score_e2_recency, score_e3_periodic, score_e4_sequence, temporal_time_bucket,
};
pub use calyx_core::{
    BoostConfig, CALYX_TEMPORAL_AP60_VIOLATION, CALYX_TEMPORAL_INVALID_BOOST_CONFIG,
    CALYX_TEMPORAL_INVALID_PERIOD, CALYX_TEMPORAL_INVALID_WINDOW, CALYX_TEMPORAL_WEIGHT_SUM,
    DecayFunction, FusionWeights, MultiAnchorMode, PeriodicOptions, RecurrenceBoostConfig,
    SequenceDirection, SequenceOptions, TemporalPolicy,
};
pub use causal_gate::{
    CausalConfidence, CausalGateEvidence, apply_causal_gate, causal_gate_mult,
    derive_causal_confidence, temporal_search_pipeline,
};
pub use recall_budget::{SlotLen, WindowRecallPolicy, WindowRecallReport};
pub use recurrence_boost::{
    RecurrenceBoostEvidence, frequency_kernel_bonus, recurrence_boost_evidence,
    recurrence_boost_from_parts, recurrence_boost_score,
};
pub use search::{
    TemporalSearchInput, TemporalSearchResult, temporal_search, temporal_search_from_primary,
    temporal_search_from_primary_with_recurrence, temporal_search_with_recall,
    temporal_search_with_recurrence, temporal_search_with_recurrence_and_recall,
    validate_primary_temporal_weight,
};
pub use window::{
    Clock, FixedClock, SystemClock, TimeWindow, count_hits_in_window, filter_hits_by_window,
};

#[cfg(test)]
mod tests;
