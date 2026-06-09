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
pub mod reranker;
pub mod search;
pub mod slot_index_map;
pub mod temporal;
mod util;

pub use error::{
    CALYX_SEXTANT_DIM_MISMATCH, CALYX_SEXTANT_EF_TOO_SMALL, CALYX_SEXTANT_GPU_PARITY_UNAVAILABLE,
    CALYX_SEXTANT_INDEX_EMPTY, CALYX_SEXTANT_NO_LENSES, CALYX_SEXTANT_PLAN_COST_EXCEEDED,
    CALYX_SEXTANT_PLAN_UNBOUNDED, CALYX_SEXTANT_POSTINGS_CORRUPT,
    CALYX_SEXTANT_POSTINGS_NOT_SORTED, CALYX_SEXTANT_PROVENANCE_MISSING,
    CALYX_SEXTANT_RERANKER_TIMEOUT, CALYX_SEXTANT_SLOT_ALREADY_REGISTERED,
    CALYX_SEXTANT_SLOT_INACTIVE, CALYX_SEXTANT_SLOT_MISSING, CALYX_SEXTANT_VECTOR_SHAPE,
    CALYX_TEMPORAL_AP60_VIOLATION, CALYX_TEMPORAL_INVALID_PERIOD, CALYX_TEMPORAL_INVALID_WINDOW,
    CALYX_TEMPORAL_WEIGHT_SUM, sextant_error,
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
pub use navigation::{LensComparison, compare_lenses, define, neighbors};
pub use planner::{IntentLabel, PlanLimits, PlannedQuery, QueryPlanner};
pub use planner_explain::PlannerExplain;
pub use query::{
    AnchorPredicate, FreshnessRequirement, MetadataPredicate, Query, QueryFilters, QueryGuard,
    ScalarOp, ScalarPredicate,
};
pub use reranker::{RerankRequest, RerankerClient};
pub use search::SearchEngine;
pub use slot_index_map::SlotIndexMap;
pub use temporal::{
    BoostConfig, DecayFunction, FixedClock as TemporalFixedClock, FusionWeights, MultiAnchorMode,
    PeriodicOptions, SequenceDirection, SequenceOptions, SystemClock as TemporalSystemClock,
    TemporalPolicy, TimeWindow, filter_hits_by_window,
};

#[cfg(test)]
mod tests {
    #[test]
    fn crate_metadata_is_present() {
        assert_eq!(env!("CARGO_PKG_NAME"), "calyx-sextant");
    }
}
