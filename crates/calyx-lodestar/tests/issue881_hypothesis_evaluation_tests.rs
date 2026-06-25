use std::fs;

use calyx_core::CxId;
use calyx_lodestar::{
    EvaluatorRun, HypothesisEvaluationInput, HypothesisEvaluationParams,
    HypothesisEvaluationVerdict, RetrievedEvidence, aggregate_hypothesis_evaluations,
};
use serde_json::json;

fn id(label: &str) -> CxId {
    CxId::from_input(label.as_bytes(), 1, b"issue881-hypothesis-evaluation")
}

#[test]
fn aggregates_multiple_prompt_temperature_runs_and_ranks() {
    let inputs = vec![
        input("h-top", 0.90, 0.85, 0.80, 0.75),
        input("h-low", 0.30, 0.35, 0.40, 0.45),
    ];

    let report =
        aggregate_hypothesis_evaluations(&inputs, &HypothesisEvaluationParams::default()).unwrap();

    assert_eq!(report.schema_version, 1);
    assert_eq!(report.input_count, 2);
    assert_eq!(report.retained_count, 1);
    assert_eq!(report.rejected_count, 1);
    assert_eq!(report.evaluations[0].hypothesis_id, "h-top");
    assert_eq!(
        report.evaluations[0].verdict,
        HypothesisEvaluationVerdict::RetainForRanking
    );
    assert_eq!(report.evaluations[0].run_count, 2);
    assert_eq!(report.evaluations[0].prompt_variant_count, 2);
    assert_eq!(report.evaluations[0].temperature_variant_count, 2);
    assert_eq!(report.evaluations[0].cited_evidence.len(), 1);
    assert!(report.evaluations[0].aggregate_score > report.evaluations[1].aggregate_score);
}

#[test]
fn needs_more_evidence_verdict_is_explicit() {
    let params = HypothesisEvaluationParams {
        min_retrieved_evidence: 2,
        ..HypothesisEvaluationParams::default()
    };

    let report =
        aggregate_hypothesis_evaluations(&[input("h-needs", 0.9, 0.9, 0.9, 0.9)], &params).unwrap();

    assert_eq!(report.needs_more_evidence_count, 1);
    assert_eq!(
        report.evaluations[0].verdict,
        HypothesisEvaluationVerdict::NeedsMoreEvidence
    );
}

#[test]
fn max_ranked_truncates_after_score_sort() {
    let params = HypothesisEvaluationParams {
        max_ranked: 1,
        ..HypothesisEvaluationParams::default()
    };
    let inputs = vec![
        input("h-top", 0.90, 0.85, 0.80, 0.75),
        input("h-second", 0.70, 0.70, 0.70, 0.70),
    ];

    let report = aggregate_hypothesis_evaluations(&inputs, &params).unwrap();

    assert_eq!(report.input_count, 2);
    assert_eq!(report.evaluations.len(), 1);
    assert_eq!(report.evaluations[0].hypothesis_id, "h-top");
}

#[test]
fn invalid_evaluator_state_fails_closed() {
    let mut one_run = input("h-one-run", 0.8, 0.8, 0.8, 0.8);
    one_run.evaluator_runs.pop();
    let err = aggregate_hypothesis_evaluations(&[one_run], &HypothesisEvaluationParams::default())
        .unwrap_err();
    assert_eq!(err.code(), "CALYX_KERNEL_INVALID_PARAMS");

    let mut bad_citation = input("h-bad-citation", 0.8, 0.8, 0.8, 0.8);
    bad_citation.evaluator_runs[0].cited_evidence_ids = vec!["missing".to_string()];
    let err =
        aggregate_hypothesis_evaluations(&[bad_citation], &HypothesisEvaluationParams::default())
            .unwrap_err();
    assert_eq!(err.code(), "CALYX_KERNEL_INVALID_PARAMS");

    let mut bad_score = input("h-bad-score", 0.8, 0.8, 0.8, 0.8);
    bad_score.evaluator_runs[0].plausible_score = f32::NAN;
    let err =
        aggregate_hypothesis_evaluations(&[bad_score], &HypothesisEvaluationParams::default())
            .unwrap_err();
    assert_eq!(err.code(), "CALYX_KERNEL_INVALID_PARAMS");
}

#[test]
fn writes_fsv_readback_when_root_is_set() {
    let Ok(root) = std::env::var("CALYX_FSV_ROOT") else {
        return;
    };
    let inputs = vec![
        input("h-top", 0.90, 0.85, 0.80, 0.75),
        input("h-low", 0.30, 0.35, 0.40, 0.45),
    ];
    let report =
        aggregate_hypothesis_evaluations(&inputs, &HypothesisEvaluationParams::default()).unwrap();
    let top = &report.evaluations[0];
    let value = json!({
        "issue": 881,
        "schema_version": report.schema_version,
        "input_count": report.input_count,
        "evaluation_count": report.evaluations.len(),
        "retained_count": report.retained_count,
        "rejected_count": report.rejected_count,
        "top_hypothesis_id": top.hypothesis_id,
        "top_aggregate_score": top.aggregate_score,
        "top_prompt_variant_count": top.prompt_variant_count,
        "top_temperature_variant_count": top.temperature_variant_count,
        "top_evidence_count": top.evidence_count,
        "full_report": report,
    });
    fs::create_dir_all(&root).unwrap();
    let path = std::path::Path::new(&root).join("issue881_hypothesis_evaluation_readback.json");
    fs::write(&path, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
    let bytes = fs::read(&path).unwrap();
    let readback: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(readback["evaluation_count"], 2);
    assert_eq!(readback["retained_count"], 1);
    assert_eq!(readback["top_prompt_variant_count"], 2);
    println!("issue881_fsv_path={} bytes={}", path.display(), bytes.len());
}

fn input(
    hypothesis_id: &str,
    plausible: f32,
    novelty: f32,
    testability: f32,
    falsifiability: f32,
) -> HypothesisEvaluationInput {
    HypothesisEvaluationInput {
        hypothesis_id: hypothesis_id.to_string(),
        a: id(&format!("{hypothesis_id}-a")),
        b: id(&format!("{hypothesis_id}-b")),
        c: id(&format!("{hypothesis_id}-c")),
        claim: format!("transparent synthetic claim for {hypothesis_id}"),
        grounded_confidence: 0.82,
        chain_provenance: vec![format!("chain_log={hypothesis_id}")],
        retrieved_evidence: vec![RetrievedEvidence {
            evidence_id: "e1".to_string(),
            source_cx_id: id(&format!("{hypothesis_id}-e1")),
            title: "grounded abstract title".to_string(),
            abstract_text: "grounded provenance abstract text".to_string(),
            grounding_confidence: 0.91,
            provenance: vec!["retrieved_from_grounded_chain".to_string()],
        }],
        evaluator_runs: vec![
            run(
                "prompt-a",
                20,
                plausible,
                novelty,
                testability,
                falsifiability,
            ),
            run(
                "prompt-b",
                70,
                plausible - 0.05,
                novelty - 0.05,
                testability - 0.05,
                falsifiability - 0.05,
            ),
        ],
    }
}

fn run(
    prompt_id: &str,
    temperature_x100: u16,
    plausible_score: f32,
    novelty_score: f32,
    testability_score: f32,
    falsifiability_score: f32,
) -> EvaluatorRun {
    EvaluatorRun {
        prompt_id: prompt_id.to_string(),
        temperature_x100,
        plausible_score,
        novelty_score,
        testability_score,
        falsifiability_score,
        justification: format!("{prompt_id} transparent justification"),
        falsification_test: format!("{prompt_id} falsification test"),
        cited_evidence_ids: vec!["e1".to_string()],
    }
}
