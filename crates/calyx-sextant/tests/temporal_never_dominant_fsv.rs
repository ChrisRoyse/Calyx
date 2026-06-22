use std::fs;

use calyx_core::{BoostConfig, CALYX_TEMPORAL_AP60_VIOLATION, DecayFunction, LedgerRef};
use calyx_sextant::{
    FreshnessTag, FusionWeights, Hit, PeriodicOptions, ProvenanceSource, TemporalFixedClock,
    TemporalPolicy, TemporalSearchInput, TimeWindow, apply_temporal_boost,
    temporal_search_from_primary, temporal_search_pipeline, validate_primary_temporal_weight,
};
use serde_json::json;

#[path = "sextant_support/mod.rs"]
mod sextant_support;
use sextant_support::{
    cx_u8_fill as cx, fsv_root, reset_dir, write_json, write_root_file_blake3_sums,
};

const QUERY_TIME: i64 = 1_000_000;

#[test]
fn temporal_never_dominant_fsv_writes_readback() {
    let (root, keep_root) = fsv_root(
        "CALYX_TEMPORAL_NEVER_DOMINANT_FSV_ROOT",
        "calyx-temporal-never-dominant-fsv",
    );
    reset_dir(&root);
    let output_path = root.join("temporal-never-dominant-readback.json");
    let before_output_exists = output_path.exists();

    let never_hits = vec![
        hit(1, 0.80, 1, Some(QUERY_TIME - 3_600)),
        hit(2, 0.60, 2, Some(QUERY_TIME - 1_800)),
        hit(3, 0.0, 3, Some(QUERY_TIME - 300)),
    ];
    let reorder_hits = vec![
        hit(10, 0.66, 1, Some(QUERY_TIME - 86_400)),
        hit(11, 0.65, 2, Some(QUERY_TIME - 600)),
    ];
    write_json(
        &root.join("temporal-never-dominant-input.json"),
        &json!({
            "clock_secs": QUERY_TIME,
            "never_dominant_hits": hit_readback(&never_hits),
            "reorder_hits": hit_readback(&reorder_hits),
            "hand_expected": {
                "content_miss_id": id_hex(3),
                "content_miss_score_after": 0.0,
                "positive_surface_ids": [id_hex(1), id_hex(2)],
                "reorder_pre_ids": [id_hex(10), id_hex(11)],
                "reorder_post_ids": [id_hex(11), id_hex(10)],
                "ap60_temporal_weight_used": 0.0,
                "e2_query_time_score": 0.5,
                "e3_utc_minus_5": 0.5,
                "e3_utc": 0.0,
            }
        }),
    );

    let never = temporal_search_pipeline(
        never_hits,
        &TimeWindow::all(),
        &step_policy(None),
        0,
        &TemporalFixedClock::new(QUERY_TIME),
    )
    .expect("never dominant");
    let reorder = temporal_search_pipeline(
        reorder_hits,
        &TimeWindow::all(),
        &exponential_policy(),
        0,
        &TemporalFixedClock::new(QUERY_TIME),
    )
    .expect("reorder");
    let ap60 = temporal_search_from_primary(TemporalSearchInput {
        primary_hits: vec![hit(20, 0.80, 1, Some(QUERY_TIME - 60))],
        temporal_weight_used: 0.0,
        final_k: 1,
        window: None,
        policy: &step_policy(None),
        clock: &TemporalFixedClock::new(QUERY_TIME),
        tz_offset_secs: 0,
        primary_slots_used: vec![calyx_core::SlotId::new(8)],
        temporal_slots_excluded: vec![calyx_core::SlotId::new(20)],
        window_recall: Default::default(),
    })
    .expect("ap60 zero");
    let invalid = validate_primary_temporal_weight(0.25).expect_err("invalid weight");
    let e2 = e2_query_time_score();
    let (e3_local, e3_utc) = e3_timezone_scores();
    let all_zero_boost = temporal_search_pipeline(
        vec![
            hit(30, 0.0, 1, Some(QUERY_TIME - 60)),
            hit(31, 0.0, 2, Some(QUERY_TIME - 30)),
        ],
        &TimeWindow::all(),
        &step_policy(None),
        0,
        &TemporalFixedClock::new(QUERY_TIME),
    )
    .expect("all zero boost");
    let all_zero_final = temporal_search_from_primary(TemporalSearchInput {
        primary_hits: vec![
            hit(30, 0.0, 1, Some(QUERY_TIME - 60)),
            hit(31, 0.0, 2, Some(QUERY_TIME - 30)),
        ],
        temporal_weight_used: 0.0,
        final_k: 2,
        window: None,
        policy: &step_policy(None),
        clock: &TemporalFixedClock::new(QUERY_TIME),
        tz_offset_secs: 0,
        primary_slots_used: vec![calyx_core::SlotId::new(8)],
        temporal_slots_excluded: vec![calyx_core::SlotId::new(20)],
        window_recall: Default::default(),
    })
    .expect("all zero final");

    let readback = json!({
        "before_output_exists": before_output_exists,
        "trigger": "temporal_search_pipeline AP-60 invariant proof suite",
        "never_dominant": {
            "after_hits": hit_readback(&never),
            "content_miss_score_after": score_for(&never, 3),
            "positive_surface_ids": positive_ids(&never),
        },
        "boost_reorder": {
            "pre_ids": [id_hex(10), id_hex(11)],
            "post_ids": ids(&reorder),
            "old_score_after": score_for(&reorder, 10),
            "fresh_score_after": score_for(&reorder, 11),
            "fresh_beats_old": score_for(&reorder, 11) > score_for(&reorder, 10),
            "max_score": reorder.iter().map(|hit| hit.score).fold(0.0_f32, f32::max),
        },
        "ap60_weight": {
            "temporal_weight_used": ap60.temporal_weight_used,
            "invalid_weight_before": 0.25,
            "invalid_weight_error": invalid.code,
            "expected_error": CALYX_TEMPORAL_AP60_VIOLATION,
        },
        "e2_query_time": {
            "event_time_secs": 1_000_000,
            "ingest_seq": 1_100_000,
            "query_time_secs": 1_200_000,
            "actual_e2": e2,
            "expected_e2": 0.5,
            "wrong_ingest_relative_e2": 0.75,
        },
        "e3_timezone": {
            "event_utc_secs": 1_704_222_000,
            "utc_minus_5": e3_local,
            "utc": e3_utc,
            "expected_utc_minus_5": 0.5,
            "expected_utc": 0.0,
        },
        "all_zero_edge": {
            "before_count": 2,
            "boost_after_hits": hit_readback(&all_zero_boost),
            "final_after_hits": hit_readback(&all_zero_final.hits),
            "after_positive_surface_count": all_zero_final.hits.len(),
        },
    });
    write_json(&output_path, &readback);
    write_root_file_blake3_sums(&root);

    println!("temporal_never_dominant_fsv_root={}", root.display());
    println!("{}", serde_json::to_string_pretty(&readback).unwrap());

    assert_eq!(score_for(&never, 3), 0.0);
    assert_eq!(positive_ids(&never), vec![id_hex(1), id_hex(2)]);
    assert_eq!(ids(&reorder), vec![id_hex(11), id_hex(10)]);
    assert_eq!(ap60.temporal_weight_used, 0.0);
    assert_eq!(invalid.code, CALYX_TEMPORAL_AP60_VIOLATION);
    assert!((e2 - 0.5).abs() <= 1.0e-5);
    assert_eq!((e3_local, e3_utc), (0.5, 0.0));
    assert!(all_zero_boost.iter().all(|hit| hit.score == 0.0));
    assert!(all_zero_final.hits.is_empty());

    if !keep_root {
        fs::remove_dir_all(root).expect("cleanup temp root");
    }
}

fn e2_query_time_score() -> f32 {
    let mut candidate = hit(40, 0.80, 1, Some(1_000_000));
    candidate.provenance.seq = 1_100_000;
    let boosted = apply_temporal_boost(vec![candidate], &linear_policy(400_000), 1_200_000, 0)
        .expect("e2 boost");
    boosted[0].temporal_scores.expect("scores").e2_recency
}

fn e3_timezone_scores() -> (f32, f32) {
    let policy = step_policy(Some(14));
    let local = apply_temporal_boost(
        vec![hit(41, 0.80, 1, Some(1_704_222_000))],
        &policy,
        1_704_222_060,
        -18_000,
    )
    .expect("local");
    let utc = apply_temporal_boost(
        vec![hit(41, 0.80, 1, Some(1_704_222_000))],
        &policy,
        1_704_222_060,
        0,
    )
    .expect("utc");
    (
        local[0].temporal_scores.expect("local scores").e3_periodic,
        utc[0].temporal_scores.expect("utc scores").e3_periodic,
    )
}

fn step_policy(target_hour: Option<u8>) -> TemporalPolicy {
    policy(DecayFunction::Step, target_hour)
}

fn exponential_policy() -> TemporalPolicy {
    policy(
        DecayFunction::Exponential {
            half_life_secs: 3_600,
        },
        None,
    )
}

fn linear_policy(max_age_secs: u64) -> TemporalPolicy {
    policy(DecayFunction::Linear { max_age_secs }, None)
}

fn policy(decay: DecayFunction, target_hour: Option<u8>) -> TemporalPolicy {
    TemporalPolicy::new(
        true,
        decay,
        PeriodicOptions::new(target_hour, None).expect("periodic"),
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
        causal_confidence: calyx_sextant::CausalConfidence::Absent,
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

fn positive_ids(hits: &[Hit]) -> Vec<String> {
    hits.iter()
        .filter(|hit| hit.score > 0.0)
        .map(|hit| hit.cx_id.to_string())
        .collect()
}

fn ids(hits: &[Hit]) -> Vec<String> {
    hits.iter().map(|hit| hit.cx_id.to_string()).collect()
}

fn hit_readback(hits: &[Hit]) -> Vec<serde_json::Value> {
    hits.iter()
        .map(|hit| {
            json!({
                "cx_id": hit.cx_id.to_string(),
                "rank": hit.rank,
                "score": hit.score,
                "event_time_secs": hit.event_time_secs,
                "temporal_scores": hit.temporal_scores,
                "causal_gate": hit.causal_gate,
            })
        })
        .collect()
}

fn id_hex(seed: u8) -> String {
    cx(seed).to_string()
}
