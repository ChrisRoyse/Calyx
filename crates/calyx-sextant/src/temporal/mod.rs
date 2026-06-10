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
mod tests {
    use super::*;

    const WEIGHT_SUM_EPSILON: f32 = 1.0e-6;

    #[test]
    fn default_fusion_weights_sum_to_one() {
        let weights = FusionWeights::default();
        let sum = weights.recency + weights.sequence + weights.periodic;
        assert!((sum - 1.0).abs() < WEIGHT_SUM_EPSILON);
        weights.validate().expect("default weights valid");
    }

    #[test]
    fn fusion_weights_fail_closed_when_sum_is_wrong() {
        assert_eq!(
            FusionWeights::new(0.4, 0.4, 0.2).expect("valid weights"),
            FusionWeights {
                recency: 0.4,
                sequence: 0.4,
                periodic: 0.2,
            }
        );
        let error = FusionWeights::new(0.4, 0.4, 0.3).expect_err("bad sum rejected");
        assert_eq!(error.code, CALYX_TEMPORAL_WEIGHT_SUM);
    }

    #[test]
    fn temporal_policy_default_roundtrips_byte_exact() {
        let policy = TemporalPolicy::default();
        let first = serde_json::to_vec(&policy).expect("serialize policy");
        let decoded: TemporalPolicy = serde_json::from_slice(&first).expect("deserialize policy");
        let second = serde_json::to_vec(&decoded).expect("serialize decoded");
        assert_eq!(first, second);
        assert_eq!(policy, decoded);
    }

    #[test]
    fn temporal_policy_deserialize_fails_closed_when_invalid() {
        let mut value = serde_json::to_value(TemporalPolicy::default()).expect("policy json");
        value["never_dominant"] = serde_json::json!(false);
        let error = serde_json::from_value::<TemporalPolicy>(value).expect_err("invalid policy");
        assert!(error.to_string().contains(CALYX_TEMPORAL_AP60_VIOLATION));
    }

    #[test]
    fn periodic_options_reject_invalid_hour_and_day() {
        let hour_error = PeriodicOptions::new(Some(24), None).expect_err("hour rejected");
        assert_eq!(hour_error.code, CALYX_TEMPORAL_INVALID_PERIOD);
        let day_error = PeriodicOptions::new(None, Some(7)).expect_err("day rejected");
        assert_eq!(day_error.code, CALYX_TEMPORAL_INVALID_PERIOD);
    }

    #[test]
    fn never_dominant_false_fails_closed() {
        let error = TemporalPolicy::new(
            true,
            DecayFunction::default(),
            PeriodicOptions::default(),
            SequenceOptions::default(),
            FusionWeights::default(),
            BoostConfig::default(),
            false,
        )
        .expect_err("AP-60 violation rejected");
        assert_eq!(error.code, CALYX_TEMPORAL_AP60_VIOLATION);
    }

    #[test]
    fn boost_config_zero_high_multiplier_is_allowed_by_type_layer() {
        let policy = TemporalPolicy::new(
            true,
            DecayFunction::default(),
            PeriodicOptions::default(),
            SequenceOptions::default(),
            FusionWeights::default(),
            BoostConfig {
                post_retrieval_alpha: 0.10,
                causal_high_mult: 0.0,
                causal_low_mult: 0.85,
            },
            true,
        )
        .expect("T01 does not enforce boost range");
        assert_eq!(policy.boost.causal_high_mult, 0.0);
    }

    #[test]
    fn zero_fusion_weights_fail_closed() {
        let error = FusionWeights::new(0.0, 0.0, 0.0).expect_err("zero sum rejected");
        assert_eq!(error.code, CALYX_TEMPORAL_WEIGHT_SUM);
    }
}
