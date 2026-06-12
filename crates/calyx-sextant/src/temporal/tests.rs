use calyx_core::{CxId, LedgerRef, SlotId};
use proptest::prelude::*;

use super::*;
use crate::hit::{FreshnessTag, Hit, ProvenanceSource};

const WEIGHT_SUM_EPSILON: f32 = 1.0e-6;
const SCORE_EPSILON: f32 = 1.0e-5;
const QUERY_TIME: i64 = 1_000_000;

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
fn boost_config_enforces_causal_multiplier_semantics() {
    let valid = TemporalPolicy::new(
        true,
        DecayFunction::default(),
        PeriodicOptions::default(),
        SequenceOptions::default(),
        FusionWeights::default(),
        BoostConfig {
            post_retrieval_alpha: 0.10,
            causal_high_mult: 1.25,
            causal_low_mult: 0.50,
        },
        true,
    )
    .expect("boost/dampen config valid");
    assert_eq!(valid.boost.causal_high_mult, 1.25);
    assert_eq!(valid.boost.causal_low_mult, 0.50);

    assert_boost_config_error(0.0, 0.85, "causal_high_mult");
    assert_boost_config_error(1.0, 0.85, "causal_high_mult");
    assert_boost_config_error(1.10, 1.0, "causal_low_mult");
    assert_boost_config_error(1.05, 1.10, "causal_low_mult");
    assert_boost_config_error(-0.5, 0.85, "causal_high_mult");
    assert_boost_config_error(f32::NAN, 0.85, "causal_high_mult");
}

#[test]
fn zero_fusion_weights_fail_closed() {
    let error = FusionWeights::new(0.0, 0.0, 0.0).expect_err("zero sum rejected");
    assert_eq!(error.code, CALYX_TEMPORAL_WEIGHT_SUM);
}

fn assert_boost_config_error(high: f32, low: f32, message: &str) {
    let error = TemporalPolicy::new(
        true,
        DecayFunction::default(),
        PeriodicOptions::default(),
        SequenceOptions::default(),
        FusionWeights::default(),
        BoostConfig {
            post_retrieval_alpha: 0.10,
            causal_high_mult: high,
            causal_low_mult: low,
        },
        true,
    )
    .expect_err("invalid boost config rejected");
    assert_eq!(error.code, CALYX_TEMPORAL_INVALID_BOOST_CONFIG);
    assert!(error.message.contains(message), "{}", error.message);
}

#[test]
fn fsv_temporal_never_dominant() {
    let hits = vec![
        hit(1, 0.80, 1, Some(QUERY_TIME - 3_600)),
        hit(2, 0.60, 2, Some(QUERY_TIME - 1_800)),
        hit(3, 0.0, 3, Some(QUERY_TIME - 300)),
    ];
    let piped = temporal_search_pipeline(
        hits,
        &TimeWindow::all(),
        &step_policy(None),
        0,
        &FixedClock::new(QUERY_TIME),
    )
    .expect("pipeline");
    let miss_score = score_for(&piped, 3);

    println!("content-miss score after boost: {miss_score}");
    println!("never-dominant final ids: {:?}", ids(&piped));
    assert!(ids(&piped).contains(&1));
    assert!(ids(&piped).contains(&2));
    assert_eq!(miss_score, 0.0);
}

#[test]
fn fsv_boost_reorders_content_matches() {
    let hits = vec![
        hit(1, 0.66, 1, Some(QUERY_TIME - 86_400)),
        hit(2, 0.65, 2, Some(QUERY_TIME - 600)),
    ];
    let piped = temporal_search_pipeline(
        hits,
        &TimeWindow::all(),
        &exponential_policy(),
        0,
        &FixedClock::new(QUERY_TIME),
    )
    .expect("pipeline");

    println!("boost reorder pre ids: [1, 2]");
    println!("boost reorder post ids: {:?}", ids(&piped));
    println!(
        "boost reorder scores: old={} fresh={}",
        score_for(&piped, 1),
        score_for(&piped, 2)
    );
    assert_eq!(ids(&piped), vec![2, 1]);
    assert!(score_for(&piped, 2) > score_for(&piped, 1));
    assert!(piped.iter().all(|hit| hit.score <= 1.10));
}

#[test]
fn fsv_ap60_weight_zero_in_retrieval() {
    let result = temporal_search_from_primary(TemporalSearchInput {
        primary_hits: vec![hit(1, 0.80, 1, Some(QUERY_TIME - 60))],
        temporal_weight_used: 0.0,
        final_k: 1,
        window: None,
        policy: &step_policy(None),
        clock: &FixedClock::new(QUERY_TIME),
        tz_offset_secs: 0,
        primary_slots_used: vec![SlotId::new(8)],
        temporal_slots_excluded: vec![SlotId::new(20)],
        window_recall: Default::default(),
    })
    .expect("zero temporal weight");
    let error = validate_primary_temporal_weight(0.25).expect_err("non-zero weight rejected");

    println!("ap60 temporal_weight_used: {}", result.temporal_weight_used);
    assert_eq!(result.temporal_weight_used, 0.0);
    assert_eq!(error.code, CALYX_TEMPORAL_AP60_VIOLATION);
}

#[test]
fn fsv_e2_uses_query_time_not_ingest_time() {
    let mut candidate = hit(1, 0.80, 1, Some(1_000_000));
    candidate.provenance.seq = 1_100_000;
    let boosted = apply_temporal_boost(vec![candidate], &linear_policy(400_000), 1_200_000, 0)
        .expect("boost");
    let e2 = boosted[0].temporal_scores.expect("scores").e2_recency;

    println!("e2 query-time score: {e2}");
    assert_close(e2, 0.5);
    assert!((e2 - 0.75).abs() > SCORE_EPSILON);
}

#[test]
fn fsv_e3_timezone_aware() {
    let policy = step_policy(Some(14));
    let local = apply_temporal_boost(
        vec![hit(1, 0.80, 1, Some(1_704_222_000))],
        &policy,
        1_704_222_060,
        -18_000,
    )
    .expect("local boost");
    let utc = apply_temporal_boost(
        vec![hit(1, 0.80, 1, Some(1_704_222_000))],
        &policy,
        1_704_222_060,
        0,
    )
    .expect("utc boost");

    let local_e3 = local[0].temporal_scores.expect("local scores").e3_periodic;
    let utc_e3 = utc[0].temporal_scores.expect("utc scores").e3_periodic;
    println!("e3 utc_minus_5={local_e3} utc={utc_e3}");
    assert_close(local_e3, 0.5);
    assert_close(utc_e3, 0.0);
}

#[test]
fn all_zero_content_has_empty_positive_surface() {
    let piped = temporal_search_pipeline(
        vec![
            hit(1, 0.0, 1, Some(QUERY_TIME - 60)),
            hit(2, 0.0, 2, Some(QUERY_TIME - 30)),
        ],
        &TimeWindow::all(),
        &step_policy(None),
        0,
        &FixedClock::new(QUERY_TIME),
    )
    .expect("pipeline");

    assert!(piped.iter().all(|hit| hit.score == 0.0));
    assert_eq!(piped.iter().filter(|hit| hit.score > 0.0).count(), 0);
}

proptest! {
    #[test]
    fn zero_content_score_remains_zero_for_any_primary_set(
        scores in proptest::collection::vec(0.01_f32..1.0, 0..16),
    ) {
        let mut hits = scores
            .iter()
            .enumerate()
            .map(|(idx, score)| hit(idx as u8, *score, idx + 1, Some(QUERY_TIME - 900)))
            .collect::<Vec<_>>();
        hits.push(hit(250, 0.0, hits.len() + 1, Some(QUERY_TIME - 1)));

        let piped = temporal_search_pipeline(
            hits,
            &TimeWindow::all(),
            &step_policy(None),
            0,
            &FixedClock::new(QUERY_TIME),
        ).expect("pipeline");

        prop_assert_eq!(score_for(&piped, 250), 0.0);
    }
}

fn step_policy(target_hour: Option<u8>) -> TemporalPolicy {
    TemporalPolicy::new(
        true,
        DecayFunction::Step,
        PeriodicOptions::new(target_hour, None).expect("periodic"),
        Default::default(),
        FusionWeights::default(),
        BoostConfig::default(),
        true,
    )
    .expect("policy")
}

fn exponential_policy() -> TemporalPolicy {
    TemporalPolicy::new(
        true,
        DecayFunction::Exponential {
            half_life_secs: 3_600,
        },
        PeriodicOptions::new(None, None).expect("periodic"),
        Default::default(),
        FusionWeights::default(),
        BoostConfig::default(),
        true,
    )
    .expect("policy")
}

fn linear_policy(max_age_secs: u64) -> TemporalPolicy {
    TemporalPolicy::new(
        true,
        DecayFunction::Linear { max_age_secs },
        PeriodicOptions::new(None, None).expect("periodic"),
        Default::default(),
        FusionWeights::default(),
        BoostConfig::default(),
        true,
    )
    .expect("policy")
}

fn hit(seed: u8, score: f32, rank: usize, event_time_secs: Option<i64>) -> Hit {
    Hit {
        cx_id: cx(seed),
        score,
        rank,
        event_time_secs,
        temporal_scores: None,
        causal_confidence: CausalConfidence::Absent,
        causal_gate: None,
        per_lens: Vec::new(),
        cross_terms_used: false,
        guard: None,
        provenance: LedgerRef {
            seq: seed as u64,
            hash: [seed; 32],
        },
        provenance_source: ProvenanceSource::Stub,
        freshness: FreshnessTag::fresh(0),
        explain: None,
    }
}

fn score_for(hits: &[Hit], seed: u8) -> f32 {
    hits.iter()
        .find(|hit| hit.cx_id == cx(seed))
        .expect("hit by seed")
        .score
}

fn ids(hits: &[Hit]) -> Vec<u8> {
    hits.iter().map(|hit| hit.cx_id.as_bytes()[0]).collect()
}

fn cx(seed: u8) -> CxId {
    CxId::from_bytes([seed; 16])
}

fn assert_close(actual: f32, expected: f32) {
    assert!(
        (actual - expected).abs() <= SCORE_EPSILON,
        "actual {actual} expected {expected}"
    );
}
