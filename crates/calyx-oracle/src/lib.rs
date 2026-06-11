//! Oracle consequence prediction and completion primitives.

mod prd22;
mod time_prediction;

pub use prd22::{
    ConsequenceExpansion, OracleCeiling, OraclePrediction, SuperIntelligenceEvidence,
    SuperIntelligenceVerdict, butterfly_expand, oracle_ceiling, oracle_predict, reverse_query,
    super_intelligence,
};
pub use time_prediction::{
    CALYX_ORACLE_INSUFFICIENT, MIN_TIME_PREDICTION_OCCURRENCES, TimePrediction,
    TimePredictionInterval, predict_next_occurrence, predict_next_occurrence_from_series,
};

#[cfg(test)]
mod tests {
    #[test]
    fn crate_metadata_is_present() {
        assert_eq!(env!("CARGO_PKG_NAME"), "calyx-oracle");
    }
}
