//! Oracle consequence prediction and completion primitives.

mod butterfly;
mod error;
mod honesty_gate;
mod prd22;
mod predict;
mod self_consistency;
mod time_prediction;
mod types;

pub use butterfly::{
    HOP_ATTENUATION, MAX_DEPTH, MIN_CONFIDENCE_THRESHOLD, build_tree, expand,
    is_provisional_ledger_ref, provisional_ledger_ref, select,
};
pub use error::{
    CALYX_ORACLE_DOMAIN_NOT_FOUND, CALYX_ORACLE_FLAKY_ANCHOR, CALYX_ORACLE_INSUFFICIENT,
    CALYX_ORACLE_LEDGER_WRITE_FAILURE, CALYX_ORACLE_NO_RECURRENCE, OracleError,
};

pub use honesty_gate::{
    SufficiencyAssay, VaultSufficiencyAssay, check_sufficiency, check_sufficiency_with_assay,
};
pub use prd22::{
    ConsequenceExpansion, OracleCeiling, OraclePrediction, SuperIntelligenceEvidence,
    SuperIntelligenceVerdict, butterfly_expand, oracle_ceiling,
    oracle_predict as oracle_formula_predict, reverse_query, super_intelligence,
};
pub use predict::{Action, ORACLE_ACTION_METADATA_KEY, oracle_predict};
pub use self_consistency::{
    MIN_FLAKINESS_PAIRS, MIN_VALIDITY_SAMPLES, ORACLE_DOMAIN_METADATA_KEY,
    ORACLE_FALLBACK_DOMAIN_METADATA_KEY, oracle_self_consistency,
};
pub use time_prediction::{
    MIN_TIME_PREDICTION_OCCURRENCES, TimeBucket, TimePrediction, TimePredictionInterval,
    predict_next_occurrence, predict_next_occurrence_from_series,
    predict_next_occurrence_from_series_with_tz_offset, predict_next_occurrence_with_tz_offset,
    time_bucket,
};
pub use types::{
    Consequence, ConsequenceTree, DEFAULT_CONSEQUENCE_TREE_MAX_DEPTH, DomainId,
    OracleSelfConsistency, Prediction, SufficiencyBound,
};

#[cfg(test)]
mod tests {
    #[test]
    fn crate_metadata_is_present() {
        assert_eq!(env!("CARGO_PKG_NAME"), "calyx-oracle");
    }
}
