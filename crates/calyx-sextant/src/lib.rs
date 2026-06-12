//! Sextant search and navigation for Calyx retrieval.

pub mod error;
pub mod fusion;
pub mod guarded;
pub mod hit;
pub mod index;
pub mod navigation;
pub mod planner;
pub mod planner_explain;
pub mod query;
pub mod query_admission;
pub mod reranker;
pub mod search;
mod search_support;
pub mod slot_index_map;
pub mod temporal;
mod util;

pub use error::{
    CALYX_SEXTANT_ASSOC_GRAPH_MISSING, CALYX_SEXTANT_CONSENSUS_INSUFFICIENT_LENSES,
    CALYX_SEXTANT_CX_MISSING, CALYX_SEXTANT_DIM_MISMATCH, CALYX_SEXTANT_EF_TOO_SMALL,
    CALYX_SEXTANT_GPU_PARITY_UNAVAILABLE, CALYX_SEXTANT_INDEX_EMPTY, CALYX_SEXTANT_NO_LENSES,
    CALYX_SEXTANT_PLAN_COST_EXCEEDED, CALYX_SEXTANT_PLAN_UNBOUNDED, CALYX_SEXTANT_POSTINGS_CORRUPT,
    CALYX_SEXTANT_POSTINGS_NOT_SORTED, CALYX_SEXTANT_PROVENANCE_MISSING, CALYX_SEXTANT_QUERY_SHAPE,
    CALYX_SEXTANT_RECURRENCE_READ_ERROR, CALYX_SEXTANT_RERANKER_ENDPOINT,
    CALYX_SEXTANT_RERANKER_NO_CANDIDATES, CALYX_SEXTANT_RERANKER_PROTOCOL,
    CALYX_SEXTANT_RERANKER_TIMEOUT, CALYX_SEXTANT_SKILL_BUDGET_EXCEEDED,
    CALYX_SEXTANT_SKILL_PAIR_NO_OVERLAP, CALYX_SEXTANT_SKILL_PARAMS, CALYX_SEXTANT_SKILL_UNKNOWN,
    CALYX_SEXTANT_SLOT_ALREADY_REGISTERED, CALYX_SEXTANT_SLOT_INACTIVE, CALYX_SEXTANT_SLOT_MISSING,
    CALYX_SEXTANT_TRAVERSE_HOPS, CALYX_SEXTANT_VECTOR_SHAPE, CALYX_TEMPORAL_AP60_VIOLATION,
    CALYX_TEMPORAL_INVALID_BOOST_CONFIG, CALYX_TEMPORAL_INVALID_PERIOD,
    CALYX_TEMPORAL_INVALID_WINDOW, CALYX_TEMPORAL_WEIGHT_SUM,
    CALYX_TEMPORAL_WINDOW_BUDGET_EXHAUSTED, sextant_error,
};
pub use fusion::{FusionContext, FusionStrategy, RrfProfile, WeightedProfile, weighted_profiles};
pub use guarded::GuardedSearchReport;
pub use hit::{
    DroppedGuardHit, FreshnessTag, Hit, HitGuardEvidence, HitGuardMode, PerLensContribution,
    ProvenanceSource,
};
pub use index::{
    DualIndex, HnswIndex, IndexSearchHit, IndexStats, InvertedIndex, MaxSimIndex, QuantConfig,
    QuantKind, SextantIndex,
};
pub use navigation::{
    ConsensusHit, ConsensusMode, ConsensusReport, LensComparison, MAX_TRAVERSE_HOPS, SkillNode,
    SkillParams, SkillTree, SlotCosine, TraverseDirection, TraversePath, TraverseStep, agree,
    compare_lenses, define, disagree, neighbors, search_skill, skills, traverse, traverse_graph,
};
pub use planner::{IntentLabel, PlanLimits, PlannedQuery, QueryPlanner};
pub use planner_explain::PlannerExplain;
pub use query::{
    AnchorPredicate, FreshnessRequirement, MetadataPredicate, Query, QueryFilters, QueryGuard,
    ScalarOp, ScalarPredicate,
};
pub use query_admission::{QueryAdmissionConfig, QueryAdmissionController, QueryAdmissionStats};
pub use reranker::{RerankCandidateText, RerankRequest, RerankResponse, RerankerClient};
pub use search::SearchEngine;
pub use slot_index_map::SlotIndexMap;
pub use temporal::{
    BoostConfig, CausalConfidence, CausalGateEvidence, DecayFunction,
    FixedClock as TemporalFixedClock, FusionWeights, MultiAnchorMode, PeriodicOptions,
    RecurrenceBoostConfig, RecurrenceBoostEvidence, SequenceDirection, SequenceOptions, SlotLen,
    SystemClock as TemporalSystemClock, TemporalPolicy, TemporalScores, TemporalSearchInput,
    TemporalSearchResult, TemporalTimeBucket, TimeWindow, WindowRecallPolicy, WindowRecallReport,
    apply_causal_gate, apply_temporal_boost, apply_temporal_boost_with_recurrence,
    causal_gate_mult, count_hits_in_window, derive_causal_confidence, filter_hits_by_window,
    frequency_kernel_bonus, fuse_temporal, recurrence_boost_evidence, recurrence_boost_from_parts,
    recurrence_boost_score, score_e2_recency, score_e3_periodic, score_e4_sequence,
    temporal_search, temporal_search_from_primary, temporal_search_from_primary_with_recurrence,
    temporal_search_pipeline, temporal_search_with_recall, temporal_search_with_recurrence,
    temporal_search_with_recurrence_and_recall, temporal_time_bucket,
    validate_primary_temporal_weight,
};

#[cfg(test)]
mod tests {
    #[test]
    fn crate_metadata_is_present() {
        assert_eq!(env!("CARGO_PKG_NAME"), "calyx-sextant");
    }
}
