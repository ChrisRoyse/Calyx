//! Issue #108 - learn which local signals improve forecast scores.
//!
//! Source of truth: persisted scored-history, versioned candidate-artifact, rollback, request, and
//! strategy-learning report JSON files, all read back before assertions are recorded.

use std::path::Path;

use calyx_poly::strategy_learning::{
    ERR_STRATEGY_LEARNING_FORBIDDEN_OBJECTIVE, ERR_STRATEGY_LEARNING_INVALID_REQUEST,
    ERR_STRATEGY_LEARNING_LOOKAHEAD, ERR_STRATEGY_LEARNING_NO_PROMOTION,
    STRATEGY_LEARNING_MIN_HELDOUT_ROWS, StrategyCandidateArtifact, StrategyChangeKind,
    StrategyComponentChange, StrategyLearningReport, StrategyLearningRequest,
    StrategyLearningStatus, StrategyScoreRow, read_strategy_learning_report,
    require_strategy_learning_promoted, run_strategy_learning_report,
};
use serde_json::{Value, json};

// calyx-shared-module: path=fsv_support.rs alias=__calyx_shared_fsv_support_rs local=support visibility=private
use crate::__calyx_shared_fsv_support_rs as support;
use support::{collect_files, hex, named_fsv_root, reset_dir, write_blake3sums, write_json};

const MIN_ROWS: usize = STRATEGY_LEARNING_MIN_HELDOUT_ROWS;
const COMPONENT_KINDS: usize = 4;

#[test]
fn issue108_strategy_learning_fsv() {
    let (root, _keep) = named_fsv_root("POLY_ISSUE108_FSV_ROOT", "poly-issue108-strategy");
    reset_dir(&root);

    let happy = happy_promotes_score_improving_strategy(&root);
    let calibration = edge_degraded_calibration_refuses_promotion(&root);
    let lookahead = edge_lookahead_fails_loud(&root);
    let insufficient = edge_insufficient_heldout_fails_loud(&root);
    let forbidden = edge_forbidden_objective_fails_loud(&root);

    let mut files = Vec::new();
    collect_files(&root, &mut files);
    let readback = json!({
        "issue": 108,
        "proof_claim": "Poly learns local forecast strategy changes from held-out resolved score history, promotes only when Brier, calibration, sufficiency, recall, attribution, and drift metrics improve, and stores provenance plus rollback evidence while refusing look-ahead leakage, insufficient data, degraded calibration, and betting objectives.",
        "minimum_sufficient_corpus": {
            "heldout_rows": MIN_ROWS,
            "component_kinds": COMPONENT_KINDS,
            "why_this_is_sufficient": "Four held-out rows are the smallest balanced two-YES/two-NO corpus that proves Brier and calibration deltas over both outcome classes; four component artifacts are exactly one each for lens, association, prompt, and calibration-feature learning.",
            "why_smaller_is_insufficient": "Fewer rows leave one outcome class underrepresented for score deltas, and fewer components omit one of the #108 signal classes.",
            "why_larger_is_wasteful": "More rows or repeated components would exercise the same persisted metric, provenance, rollback, and refusal gates without adding a new #108 behavior."
        },
        "happy_path": happy,
        "edge_cases": {
            "degraded_calibration": calibration,
            "lookahead_leakage": lookahead,
            "insufficient_heldout": insufficient,
            "forbidden_objective": forbidden
        },
        "physical_files": files
    });
    let readback_path = root.join("readback.json");
    write_json(&readback_path, &readback);
    write_blake3sums(&root);
    println!(
        "ISSUE108_STRATEGY_LEARNING_READBACK={}",
        readback_path.display()
    );
}

fn happy_promotes_score_improving_strategy(root: &Path) -> Value {
    let request = request_with_sources(
        root,
        "happy",
        improving_rows(),
        950,
        vec!["optimize local Brier, calibration, sufficiency, recall, attribution, and drift metrics only".to_string()],
    );
    let report = run_and_read(root, "happy", &request);
    require_strategy_learning_promoted(&report).expect("strategy promoted");
    assert_eq!(report.status, StrategyLearningStatus::Promoted);
    assert_eq!(report.heldout_count, MIN_ROWS);
    assert_eq!(report.positive_count, 2);
    assert_eq!(report.candidate.components.len(), COMPONENT_KINDS);
    assert!(metric(&report, "brier").improvement > 0.18);
    assert!(metric(&report, "calibration_abs_error").improvement > 0.24);
    assert!(report.promoted_change_hash.is_some());
    json!({
        "status": report.status,
        "promotion_code": report.promotion_code,
        "brier_delta": metric(&report, "brier"),
        "calibration_delta": metric(&report, "calibration_abs_error"),
        "component_kinds": report.candidate.components,
        "rollback_hash": report.candidate.rollback_hash,
        "promoted_change_hash": report.promoted_change_hash,
        "report_hash": report.report_hash
    })
}

fn edge_degraded_calibration_refuses_promotion(root: &Path) -> Value {
    let request = request_with_sources(
        root,
        "edge-calibration",
        calibration_degraded_rows(),
        950,
        vec!["optimize local forecast score metrics only".to_string()],
    );
    let report = run_and_read(root, "edge-calibration", &request);
    assert_eq!(report.status, StrategyLearningStatus::Rejected);
    assert_eq!(report.promotion_code, "degraded_calibration");
    let err = require_strategy_learning_promoted(&report).expect_err("promotion refused");
    assert_eq!(err.code(), ERR_STRATEGY_LEARNING_NO_PROMOTION);
    json!({
        "status": report.status,
        "promotion_code": report.promotion_code,
        "brier_delta": metric(&report, "brier"),
        "calibration_delta": metric(&report, "calibration_abs_error"),
        "fail_loud_code": err.code()
    })
}

fn edge_lookahead_fails_loud(root: &Path) -> Value {
    let request = request_with_sources(
        root,
        "edge-lookahead",
        improving_rows(),
        1_500,
        vec!["optimize local forecast score metrics only".to_string()],
    );
    let err = run_strategy_learning_report(&request, &root.join("edge-lookahead"))
        .expect_err("lookahead rejected");
    assert_eq!(err.code(), ERR_STRATEGY_LEARNING_LOOKAHEAD);
    json!({"code": err.code(), "message": err.message()})
}

fn edge_insufficient_heldout_fails_loud(root: &Path) -> Value {
    let request = request_with_sources(
        root,
        "edge-insufficient",
        improving_rows()[..3].to_vec(),
        950,
        vec!["optimize local forecast score metrics only".to_string()],
    );
    let err = run_strategy_learning_report(&request, &root.join("edge-insufficient"))
        .expect_err("insufficient held-out rows rejected");
    assert_eq!(err.code(), ERR_STRATEGY_LEARNING_INVALID_REQUEST);
    json!({"code": err.code(), "message": err.message()})
}

fn edge_forbidden_objective_fails_loud(root: &Path) -> Value {
    let request = request_with_sources(
        root,
        "edge-forbidden",
        improving_rows(),
        950,
        vec!["maximize pnl and capital growth".to_string()],
    );
    let err = run_strategy_learning_report(&request, &root.join("edge-forbidden"))
        .expect_err("forbidden objective rejected");
    assert_eq!(err.code(), ERR_STRATEGY_LEARNING_FORBIDDEN_OBJECTIVE);
    json!({"code": err.code(), "message": err.message()})
}

fn run_and_read(
    root: &Path,
    dir: &str,
    request: &StrategyLearningRequest,
) -> StrategyLearningReport {
    let run =
        run_strategy_learning_report(request, &root.join(dir)).expect("strategy learning run");
    let readback = read_strategy_learning_report(&run.report_path).expect("read strategy report");
    assert_eq!(readback, run.report);
    readback
}

fn request_with_sources(
    root: &Path,
    dir: &str,
    rows: Vec<StrategyScoreRow>,
    effective_at: u64,
    objective_notes: Vec<String>,
) -> StrategyLearningRequest {
    let case_dir = root.join(dir);
    let history_path = case_dir.join("scored_history.json");
    write_json(
        &history_path,
        &serde_json::to_value(&rows).expect("history json"),
    );
    let history_readback: Vec<StrategyScoreRow> =
        serde_json::from_slice(&std::fs::read(&history_path).expect("read history"))
            .expect("decode history");
    assert_eq!(history_readback, rows);

    let candidate_spec_path = case_dir.join("candidate_strategy_v2.json");
    write_json(
        &candidate_spec_path,
        &json!({"strategy": "issue108_candidate", "components": components()}),
    );
    let rollback_path = case_dir.join("rollback_strategy_v1.json");
    write_json(&rollback_path, &json!({"restore": "baseline_strategy_v1"}));

    let candidate = StrategyCandidateArtifact {
        candidate_id: "issue108_candidate".to_string(),
        artifact_version: "v2".to_string(),
        artifact_path: candidate_spec_path.display().to_string(),
        artifact_hash: hash_file(&candidate_spec_path),
        rollback_artifact_path: rollback_path.display().to_string(),
        rollback_hash: hash_file(&rollback_path),
        created_at: 900,
        effective_at,
        provenance: vec![
            history_path.display().to_string(),
            candidate_spec_path.display().to_string(),
        ],
        objective_notes,
        components: components(),
    };
    let request = StrategyLearningRequest {
        domain: "crypto".to_string(),
        horizon_bucket: "1h_24h".to_string(),
        score_history_artifact: history_path.display().to_string(),
        candidate,
        heldout_rows: rows,
        min_heldout_rows: MIN_ROWS,
        min_brier_improvement: 0.02,
    };
    let request_path = case_dir.join("strategy_learning_request.json");
    write_json(
        &request_path,
        &serde_json::to_value(&request).expect("request json"),
    );
    request
}

fn components() -> Vec<StrategyComponentChange> {
    vec![
        component(StrategyChangeKind::Lens, "macro_event_text_bm25", "brier"),
        component(
            StrategyChangeKind::Association,
            "holder_cluster_edge",
            "recall_ratio",
        ),
        component(
            StrategyChangeKind::Prompt,
            "deepseek_prompt_v3_local",
            "calibration_abs_error",
        ),
        component(
            StrategyChangeKind::CalibrationFeature,
            "regime_platt_slope",
            "calibration_abs_error",
        ),
    ]
}

fn component(
    kind: StrategyChangeKind,
    key: &str,
    expected_metric: &str,
) -> StrategyComponentChange {
    StrategyComponentChange {
        kind,
        key: key.to_string(),
        expected_metric: expected_metric.to_string(),
    }
}

fn improving_rows() -> Vec<StrategyScoreRow> {
    vec![
        row("s108a", 1_000, true, 0.50, 0.75),
        row("s108b", 1_100, false, 0.50, 0.25),
        row("s108c", 1_200, true, 0.50, 0.75),
        row("s108d", 1_300, false, 0.50, 0.25),
    ]
}

fn calibration_degraded_rows() -> Vec<StrategyScoreRow> {
    vec![
        row("s108cal1", 1_000, true, 0.125, 0.4375),
        row("s108cal2", 1_100, false, 0.125, 0.5625),
        row("s108cal3", 1_200, true, 0.125, 0.4375),
        row("s108cal4", 1_300, false, 0.125, 0.5625),
    ]
}

fn row(
    id: &str,
    forecast_ts: u64,
    outcome: bool,
    baseline_p: f64,
    candidate_p: f64,
) -> StrategyScoreRow {
    StrategyScoreRow {
        forecast_id: id.to_string(),
        forecast_ts,
        resolved_ts: forecast_ts + 100,
        scored_ts: forecast_ts + 200,
        outcome,
        baseline_p,
        candidate_p,
        baseline_sufficiency_bits: 0.50,
        candidate_sufficiency_bits: 0.75,
        baseline_recall_ratio: 0.875,
        candidate_recall_ratio: 0.9375,
        baseline_attribution_bits: 0.125,
        candidate_attribution_bits: 0.1875,
        baseline_drift_score: 0.50,
        candidate_drift_score: 0.25,
    }
}

fn metric<'a>(
    report: &'a StrategyLearningReport,
    name: &str,
) -> &'a calyx_poly::StrategyMetricDelta {
    report
        .metric_deltas
        .iter()
        .find(|delta| delta.metric == name)
        .expect("metric delta")
}

fn hash_file(path: &Path) -> String {
    hex(blake3::hash(&std::fs::read(path).expect("hash file")).as_bytes())
}
