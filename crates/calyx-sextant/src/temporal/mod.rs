//! Temporal search policy types for AP-60 post-retrieval boosting.

mod boost;
mod causal_gate;
mod search;
mod window;

pub use boost::{
    TemporalScores, apply_temporal_boost, fuse_temporal, score_e2_recency, score_e3_periodic,
    score_e4_sequence,
};
pub use calyx_core::{
    BoostConfig, CALYX_TEMPORAL_AP60_VIOLATION, CALYX_TEMPORAL_INVALID_BOOST_CONFIG,
    CALYX_TEMPORAL_INVALID_PERIOD, CALYX_TEMPORAL_INVALID_WINDOW, CALYX_TEMPORAL_WEIGHT_SUM,
    DecayFunction, FusionWeights, MultiAnchorMode, PeriodicOptions, SequenceDirection,
    SequenceOptions, TemporalPolicy,
};
pub use causal_gate::{
    CausalConfidence, CausalGateEvidence, apply_causal_gate, causal_gate_mult,
    derive_causal_confidence, temporal_search_pipeline,
};
pub use search::{
    TemporalSearchInput, TemporalSearchResult, temporal_search, temporal_search_from_primary,
    validate_primary_temporal_weight,
};
pub use window::{Clock, FixedClock, SystemClock, TimeWindow, filter_hits_by_window};

#[cfg(test)]
mod tests;
