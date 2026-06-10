use std::fs;
use std::path::{Path, PathBuf};

use calyx_core::{
    BoostConfig, CALYX_TEMPORAL_INVALID_BOOST_CONFIG, CxId, DecayFunction, FusionWeights, LedgerRef,
};
use calyx_sextant::{
    CausalConfidence, FreshnessTag, Hit, PeriodicOptions, ProvenanceSource, TemporalFixedClock,
    TemporalPolicy, TimeWindow, apply_causal_gate, temporal_search_pipeline,
};
use serde_json::json;

const SCORE_EPSILON: f32 = 1.0e-5;

#[test]
fn causal_gate_fsv_writes_pipeline_readback() {
    let (root, keep_root) = fsv_root();
    reset_dir(&root);
    let output_path = root.join("causal-gate-readback.json");
    let before_output_exists = output_path.exists();

    let clock = TemporalFixedClock::new(1_000_000);
    let window = TimeWindow::last_hours(1, &clock).expect("window");
    let policy = temporal_policy();
    let hits = vec![
        hit(1, 0.90, Some(999_500), 1, CausalConfidence::High),
        hit(2, 0.80, Some(999_000), 2, CausalConfidence::Neutral),
        hit(3, 0.70, Some(998_000), 3, CausalConfidence::Low),
    ];
    let expected = expected_scores();
    write_json(
        &root.join("causal-gate-input.json"),
        &json!({
            "clock_secs": clock.secs,
            "window": window,
            "policy": policy,
            "hand_expected": expected,
            "input_hits": hit_readback(&hits),
        }),
    );

    let piped =
        temporal_search_pipeline(hits.clone(), &window, &policy, 0, &clock).expect("pipeline");
    let empty = apply_causal_gate(Vec::new(), &policy.boost).expect("empty edge");
    let absent = apply_causal_gate(
        vec![hit(4, 0.42, Some(999_900), 1, CausalConfidence::Absent)],
        &policy.boost,
    )
    .expect("absent edge");
    let invalid_negative = BoostConfig {
        causal_high_mult: -0.5,
        ..policy.boost
    };
    let invalid_high = apply_causal_gate(Vec::new(), &invalid_negative)
        .expect_err("negative multiplier fails closed");
    let invalid_over_max = BoostConfig {
        causal_low_mult: 10.5,
        ..policy.boost
    };
    let invalid_low = apply_causal_gate(Vec::new(), &invalid_over_max)
        .expect_err("over-max multiplier fails closed");

    let readback = json!({
        "before_output_exists": before_output_exists,
        "trigger": "temporal_search_pipeline(window -> apply_temporal_boost -> apply_causal_gate)",
        "actual_hits": hit_readback(&piped),
        "expected_scores": expected,
        "high_score_matches": close(score_for(&piped, 1), expected.high_final),
        "neutral_score_matches": close(score_for(&piped, 2), expected.neutral_final),
        "low_score_matches": close(score_for(&piped, 3), expected.low_final),
        "explain_contains_causal_confidence": piped.iter().all(|hit| hit.causal_gate.is_some()),
        "edge_empty": {
            "before_count": 0,
            "after_count": empty.len(),
        },
        "edge_absent_is_neutral": {
            "before_confidence": "absent",
            "before_score": 0.42,
            "after_score": absent.first().map(|hit| hit.score),
            "after_gate": absent.first().and_then(|hit| hit.causal_gate),
            "expected_multiplier": 1.0,
        },
        "edge_invalid_negative": {
            "before_causal_high_mult": -0.5,
            "after_error_code": invalid_high.code,
            "expected_error_code": CALYX_TEMPORAL_INVALID_BOOST_CONFIG,
        },
        "edge_invalid_over_max": {
            "before_causal_low_mult": 10.5,
            "after_error_code": invalid_low.code,
            "expected_error_code": CALYX_TEMPORAL_INVALID_BOOST_CONFIG,
        },
    });
    write_json(&output_path, &readback);
    write_blake3_sums(&root);

    println!("causal_gate_fsv_root={}", root.display());
    println!("{}", serde_json::to_string_pretty(&readback).unwrap());

    assert_eq!(ids(&piped), vec![id_hex(1), id_hex(2), id_hex(3)]);
    assert!(close(score_for(&piped, 1), expected.high_final));
    assert!(close(score_for(&piped, 2), expected.neutral_final));
    assert!(close(score_for(&piped, 3), expected.low_final));
    assert!(piped.iter().all(|hit| hit.causal_gate.is_some()));
    assert!(empty.is_empty());
    assert_eq!(absent[0].causal_gate.map(|gate| gate.multiplier), Some(1.0));
    assert_eq!(invalid_high.code, CALYX_TEMPORAL_INVALID_BOOST_CONFIG);
    assert_eq!(invalid_low.code, CALYX_TEMPORAL_INVALID_BOOST_CONFIG);

    if !keep_root {
        fs::remove_dir_all(root).expect("cleanup temp root");
    }
}

#[derive(Clone, Copy, serde::Serialize)]
struct ExpectedScores {
    high_temporal_boosted: f32,
    high_final: f32,
    neutral_temporal_boosted: f32,
    neutral_final: f32,
    low_temporal_boosted: f32,
    low_final: f32,
    formula: &'static str,
}

fn expected_scores() -> ExpectedScores {
    ExpectedScores {
        high_temporal_boosted: 0.9675,
        high_final: 1.06425,
        neutral_temporal_boosted: 0.8506667,
        neutral_final: 0.8506667,
        low_temporal_boosted: 0.7361667,
        low_final: 0.62574166,
        formula: "(content_score + content_score * temporal_fused * 0.10) * causal_multiplier",
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

fn hit(
    seed: u8,
    score: f32,
    event_time_secs: Option<i64>,
    rank: usize,
    confidence: CausalConfidence,
) -> Hit {
    Hit {
        cx_id: CxId::from_bytes([seed; 16]),
        score,
        rank,
        event_time_secs,
        temporal_scores: None,
        causal_confidence: confidence,
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

fn hit_readback(hits: &[Hit]) -> Vec<serde_json::Value> {
    hits.iter()
        .map(|hit| {
            json!({
                "cx_id": hit.cx_id.to_string(),
                "rank": hit.rank,
                "score": hit.score,
                "event_time_secs": hit.event_time_secs,
                "temporal_scores": hit.temporal_scores,
                "causal_confidence": hit.causal_confidence,
                "causal_gate": hit.causal_gate,
            })
        })
        .collect()
}

fn score_for(hits: &[Hit], seed: u8) -> f32 {
    hits.iter()
        .find(|hit| hit.cx_id == CxId::from_bytes([seed; 16]))
        .expect("hit by id")
        .score
}

fn close(actual: f32, expected: f32) -> bool {
    (actual - expected).abs() <= SCORE_EPSILON
}

fn ids(hits: &[Hit]) -> Vec<String> {
    hits.iter().map(|hit| hit.cx_id.to_string()).collect()
}

fn id_hex(seed: u8) -> String {
    CxId::from_bytes([seed; 16]).to_string()
}

fn write_json(path: &Path, value: &serde_json::Value) {
    fs::write(path, serde_json::to_vec_pretty(value).expect("json")).expect("write json");
}

fn write_blake3_sums(root: &Path) {
    let mut entries = fs::read_dir(root)
        .expect("read root")
        .map(|entry| entry.expect("entry").path())
        .filter(|path| path.is_file())
        .collect::<Vec<_>>();
    entries.sort();
    let mut lines = String::new();
    for path in entries {
        if path.file_name().and_then(|name| name.to_str()) == Some("BLAKE3SUMS.txt") {
            continue;
        }
        let bytes = fs::read(&path).expect("read artifact");
        let name = path.file_name().expect("file name").to_string_lossy();
        lines.push_str(&format!("{}  {}\n", blake3_hex(&bytes), name));
    }
    fs::write(root.join("BLAKE3SUMS.txt"), lines).expect("write checksums");
}

fn blake3_hex(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

fn fsv_root() -> (PathBuf, bool) {
    if let Ok(root) = std::env::var("CALYX_CAUSAL_GATE_FSV_ROOT") {
        return (PathBuf::from(root), true);
    }
    (
        std::env::temp_dir().join(format!("calyx-causal-gate-fsv-{}", std::process::id())),
        false,
    )
}

fn reset_dir(dir: &Path) {
    let _ = fs::remove_dir_all(dir);
    fs::create_dir_all(dir).expect("create fsv root");
}
