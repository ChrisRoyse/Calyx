use std::fs;

#[path = "sextant_support/mod.rs"]
mod sextant_support;
use calyx_core::{BoostConfig, CALYX_TEMPORAL_AP60_VIOLATION, CxId, DecayFunction, LedgerRef};
use calyx_sextant::{
    FreshnessTag, FusionWeights, Hit, PeriodicOptions, ProvenanceSource, TemporalPolicy,
    TemporalScores, apply_temporal_boost,
};
use serde_json::json;
use sextant_support::{fsv_root, reset_dir, write_json, write_root_file_blake3_sums};

#[test]
fn temporal_boost_fsv_writes_ranked_readback() {
    let (root, keep_root) = fsv_root("CALYX_TEMPORAL_BOOST_FSV_ROOT", "calyx-temporal-boost-fsv");
    reset_dir(&root);
    let output_path = root.join("temporal-boost-readback.json");
    let before_output_exists = output_path.exists();

    let query_time_secs = 1_000_000;
    let policy = temporal_policy();
    let hits = vec![
        hit(1, 0.95, Some(900_000), 1),
        hit(2, 0.80, Some(999_500), 2),
        hit(3, 0.0, Some(999_900), 3),
    ];
    write_json(
        &root.join("temporal-boost-input.json"),
        &json!({
            "query_time_secs": query_time_secs,
            "policy": policy,
            "hand_expected": {
                "pre_rank_1": id_hex(1),
                "post_rank_1": id_hex(1),
                "zero_content_id": id_hex(3),
                "zero_content_score_after": 0.0
            },
            "input_hits": hit_readback(&hits),
        }),
    );

    let boosted = apply_temporal_boost(hits.clone(), &policy, query_time_secs, 0).expect("boost");
    let empty_boosted =
        apply_temporal_boost(Vec::new(), &policy, query_time_secs, 0).expect("empty");
    let single_boosted = apply_temporal_boost(
        vec![hit(4, 0.70, Some(999_900), 1)],
        &policy,
        query_time_secs,
        0,
    )
    .expect("single");
    let missing_time_boosted =
        apply_temporal_boost(vec![hit(5, 0.60, None, 1)], &policy, query_time_secs, 0)
            .expect("missing time");
    let zero_boosted = boosted
        .iter()
        .find(|hit| hit.cx_id == CxId::from_bytes([3; 16]))
        .expect("zero hit");

    let mut invalid_policy = policy;
    invalid_policy.never_dominant = false;
    let invalid_error = apply_temporal_boost(hits, &invalid_policy, query_time_secs, 0)
        .expect_err("invalid policy");

    let readback = json!({
        "before_output_exists": before_output_exists,
        "pre_rank_1": id_hex(1),
        "post_rank_1": boosted.first().map(|hit| hit.cx_id.to_string()),
        "post_hits": hit_readback(&boosted),
        "high_content_still_first": boosted.first().map(|hit| hit.cx_id) == Some(CxId::from_bytes([1; 16])),
        "temporal_scores_visible": boosted.iter().all(|hit| hit.temporal_scores.is_some()),
        "zero_content_edge": {
            "before_score": 0.0,
            "after_score": zero_boosted.score,
            "temporal_scores": zero_boosted.temporal_scores,
            "expected_scores": TemporalScores::zero(),
        },
        "empty_edge": {
            "before_count": 0,
            "after_count": empty_boosted.len(),
        },
        "single_edge": {
            "before_rank": 1,
            "after_rank": single_boosted.first().map(|hit| hit.rank),
            "e4_sequence": single_boosted
                .first()
                .and_then(|hit| hit.temporal_scores)
                .map(|scores| scores.e4_sequence),
        },
        "missing_time_edge": {
            "before_event_time_secs": null,
            "after_scores": missing_time_boosted.first().and_then(|hit| hit.temporal_scores),
        },
        "invalid_policy_edge": {
            "before_never_dominant": false,
            "after_error_code": invalid_error.code,
            "expected_error_code": CALYX_TEMPORAL_AP60_VIOLATION,
        }
    });
    write_json(&output_path, &readback);
    write_root_file_blake3_sums(&root);

    println!("temporal_boost_fsv_root={}", root.display());
    println!("{}", serde_json::to_string_pretty(&readback).unwrap());

    assert_eq!(
        boosted.first().map(|hit| hit.cx_id),
        Some(CxId::from_bytes([1; 16]))
    );
    assert_eq!(zero_boosted.score, 0.0);
    assert_eq!(zero_boosted.temporal_scores, Some(TemporalScores::zero()));
    assert!(empty_boosted.is_empty());
    assert_eq!(
        single_boosted[0]
            .temporal_scores
            .map(|scores| scores.e4_sequence),
        Some(1.0)
    );
    assert_eq!(invalid_error.code, CALYX_TEMPORAL_AP60_VIOLATION);

    if !keep_root {
        fs::remove_dir_all(root).expect("cleanup temp root");
    }
}

fn temporal_policy() -> TemporalPolicy {
    TemporalPolicy::new(
        true,
        DecayFunction::Step,
        PeriodicOptions::new(None, None).expect("periodic"),
        Default::default(),
        FusionWeights::default(),
        BoostConfig::default(),
        true,
    )
    .expect("policy")
}

fn hit(seed: u8, score: f32, event_time_secs: Option<i64>, rank: usize) -> Hit {
    Hit {
        cx_id: CxId::from_bytes([seed; 16]),
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

fn id_hex(seed: u8) -> String {
    CxId::from_bytes([seed; 16]).to_string()
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
            })
        })
        .collect()
}
