use std::fs;

use calyx_core::CxId;
use calyx_lodestar::{RankedHypothesisParams, TraceableHypothesisInput, rank_traceable_hypotheses};
use serde_json::json;

fn id(label: &str) -> CxId {
    CxId::from_input(label.as_bytes(), 1, b"issue882-ranked-hypotheses")
}

#[test]
fn ranks_surviving_hypotheses_and_flags_human_review() {
    let inputs = vec![
        input("h-top", 0.90, 0.85, 5, 0.88),
        input("h-mid", 0.70, 0.80, 3, 0.75),
        input("h-low", 0.30, 0.55, 2, 0.40),
    ];
    let params = RankedHypothesisParams {
        review_top_n: 2,
        min_review_score: 0.60,
        ..RankedHypothesisParams::default()
    };

    let report = rank_traceable_hypotheses(&inputs, &params).unwrap();

    assert_eq!(report.schema_version, 1);
    assert_eq!(report.input_count, 3);
    assert_eq!(report.ranked_count, 3);
    assert_eq!(report.human_review_count, 2);
    assert_eq!(report.hypotheses[0].rank, 1);
    assert_eq!(report.hypotheses[0].hypothesis_id, "h-top");
    assert!(report.hypotheses[0].human_review_flag);
    assert!(report.hypotheses[0].rank_score > report.hypotheses[1].rank_score);
    assert_eq!(report.hypotheses[2].hypothesis_id, "h-low");
    assert!(!report.hypotheses[2].human_review_flag);
}

#[test]
fn max_ranked_truncates_after_sorting() {
    let params = RankedHypothesisParams {
        max_ranked: 1,
        review_top_n: 10,
        min_review_score: 0.0,
    };
    let inputs = vec![
        input("h-top", 0.90, 0.85, 5, 0.88),
        input("h-second", 0.80, 0.80, 4, 0.80),
    ];

    let report = rank_traceable_hypotheses(&inputs, &params).unwrap();

    assert_eq!(report.input_count, 2);
    assert_eq!(report.ranked_count, 1);
    assert_eq!(report.hypotheses[0].hypothesis_id, "h-top");
    assert_eq!(report.human_review_count, 1);
}

#[test]
fn invalid_inputs_fail_closed() {
    let err = rank_traceable_hypotheses(&[], &RankedHypothesisParams::default()).unwrap_err();
    assert_eq!(err.code(), "CALYX_KERNEL_INVALID_PARAMS");

    let mut no_distance = input("h-no-distance", 0.8, 0.8, 1, 0.8);
    no_distance.cross_domain_distance = 0;
    let err =
        rank_traceable_hypotheses(&[no_distance], &RankedHypothesisParams::default()).unwrap_err();
    assert_eq!(err.code(), "CALYX_KERNEL_INVALID_PARAMS");

    let mut no_proof = input("h-no-proof", 0.8, 0.8, 1, 0.8);
    no_proof.sufficiency_proof.clear();
    let err =
        rank_traceable_hypotheses(&[no_proof], &RankedHypothesisParams::default()).unwrap_err();
    assert_eq!(err.code(), "CALYX_KERNEL_INVALID_PARAMS");

    let mut bad_score = input("h-bad-score", 0.8, 0.8, 1, 0.8);
    bad_score.novelty_score = f32::NAN;
    let err =
        rank_traceable_hypotheses(&[bad_score], &RankedHypothesisParams::default()).unwrap_err();
    assert_eq!(err.code(), "CALYX_KERNEL_INVALID_PARAMS");
}

#[test]
fn writes_fsv_readback_when_root_is_set() {
    let Ok(root) = std::env::var("CALYX_FSV_ROOT") else {
        return;
    };
    let inputs = vec![
        input("h-top", 0.90, 0.85, 5, 0.88),
        input("h-mid", 0.70, 0.80, 3, 0.75),
        input("h-low", 0.30, 0.55, 2, 0.40),
    ];
    let params = RankedHypothesisParams {
        review_top_n: 2,
        min_review_score: 0.60,
        ..RankedHypothesisParams::default()
    };
    let report = rank_traceable_hypotheses(&inputs, &params).unwrap();
    let top = &report.hypotheses[0];
    let value = json!({
        "issue": 882,
        "schema_version": report.schema_version,
        "input_count": report.input_count,
        "ranked_count": report.ranked_count,
        "human_review_count": report.human_review_count,
        "top_hypothesis_id": top.hypothesis_id,
        "top_rank": top.rank,
        "top_rank_score": top.rank_score,
        "top_human_review_flag": top.human_review_flag,
        "top_evidence_count": top.evidence_ids.len(),
        "full_report": report,
    });
    fs::create_dir_all(&root).unwrap();
    let path = std::path::Path::new(&root).join("issue882_ranked_hypotheses_readback.json");
    fs::write(&path, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
    let bytes = fs::read(&path).unwrap();
    let readback: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(readback["ranked_count"], 3);
    assert_eq!(readback["human_review_count"], 2);
    assert_eq!(readback["top_hypothesis_id"], "h-top");
    println!("issue882_fsv_path={} bytes={}", path.display(), bytes.len());
}

fn input(
    hypothesis_id: &str,
    novelty_score: f32,
    grounded_confidence: f32,
    cross_domain_distance: usize,
    evaluator_plausibility_score: f32,
) -> TraceableHypothesisInput {
    TraceableHypothesisInput {
        hypothesis_id: hypothesis_id.to_string(),
        a: id(&format!("{hypothesis_id}-a")),
        b: id(&format!("{hypothesis_id}-b")),
        c: id(&format!("{hypothesis_id}-c")),
        claim: format!("ranked synthetic claim for {hypothesis_id}"),
        novelty_score,
        grounded_confidence,
        cross_domain_distance,
        evaluator_plausibility_score,
        evaluator_aggregate_score: evaluator_plausibility_score,
        sufficiency_proof: format!("sufficiency proof for {hypothesis_id}"),
        provenance: vec![format!("chain={hypothesis_id}")],
        evidence_ids: vec![format!("{hypothesis_id}-e1")],
    }
}
