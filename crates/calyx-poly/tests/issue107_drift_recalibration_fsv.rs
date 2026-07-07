//! Issue #107 - drift-triggered forecast recalibration loop.
//!
//! Source of truth: persisted drift metrics, calibration-refit artifact, admission config, and
//! recalibration report JSON files, each read back from disk before assertions are recorded.

use std::path::{Path, PathBuf};

use calyx_poly::{
    AdmissionConfigSnapshot, CalibrationRefitObservation, CalibrationRefitReport,
    CalibrationRefitRequest, DriftMetricWindow, DriftRecalibrationReport,
    DriftRecalibrationRequest, DriftRecalibrationStatus, DriftRecalibrationThresholds,
    ERR_DRIFT_RECALIBRATION_INSUFFICIENT_ANCHORS, ERR_DRIFT_RECALIBRATION_INVALID_REQUEST,
    read_calibration_refit_report, read_drift_recalibration_report, run_calibration_refit,
    run_drift_recalibration_report,
};
use serde_json::{Value, json};

#[path = "fsv_support.rs"]
mod support;
use support::{collect_files, named_fsv_root, reset_dir, write_blake3sums, write_json};

const AS_OF: u64 = 1_785_500_000_000;
const CALIBRATION_ROWS: usize = 30;

#[test]
fn issue107_drift_recalibration_fsv() {
    let (root, _keep) = named_fsv_root("POLY_ISSUE107_FSV_ROOT", "poly-issue107-drift");
    reset_dir(&root);

    let happy = happy_drift_triggers_recalibration(&root);
    let empty = edge_empty_history_fails_loud(&root);
    let malformed = edge_malformed_window_fails_loud(&root);
    let anchors = edge_insufficient_anchors_fails_loud(&root);

    let mut files = Vec::new();
    collect_files(&root, &mut files);
    let readback = json!({
        "issue": 107,
        "proof_claim": "Poly reads local drift metrics, a versioned calibration-refit artifact, and forecast-admission config, then emits a reproducible recalibration record when read-only input drift, association recall, calibration residual, source drift, or outcome scoring thresholds fire; empty, malformed, and under-anchored windows fail closed.",
        "minimum_sufficient_corpus": {
            "calibration_observations": CALIBRATION_ROWS,
            "drift_metric_windows": 1,
            "why_this_is_sufficient": "The calibration refit already requires exactly 30 resolved observations; one latest drift window is sufficient to prove every threshold trigger and the versioned recalibration readback.",
            "why_smaller_is_insufficient": "Fewer than 30 calibration observations cannot produce the source calibration artifact, and zero drift windows cannot prove a trigger.",
            "why_larger_is_wasteful": "Additional calibration rows or windows would repeat the same refit, threshold, admission-snapshot, persistence, and readback paths without proving a new #107 behavior."
        },
        "happy_path": happy,
        "edge_cases": {
            "empty_history": empty,
            "malformed_metric_window": malformed,
            "insufficient_anchors": anchors
        },
        "physical_files": files
    });
    let readback_path = root.join("readback.json");
    write_json(&readback_path, &readback);
    write_blake3sums(&root);
    println!(
        "ISSUE107_DRIFT_RECALIBRATION_READBACK={}",
        readback_path.display()
    );
}

fn happy_drift_triggers_recalibration(root: &Path) -> Value {
    let request = request_with_sources(root, "happy", vec![drift_window(50)], thresholds());
    let report = run_and_read(root, "happy", &request);
    assert_eq!(report.status, DriftRecalibrationStatus::Triggered);
    assert_eq!(report.trigger_reasons.len(), 5);
    assert_eq!(
        report.new_calibration_version,
        request.calibration_refit.version
    );
    assert_eq!(report.calibration_observation_count, CALIBRATION_ROWS);
    assert!(report.admission_after.min_p_win > report.admission_before.min_p_win);
    assert!(report.admission_after.target_far < report.admission_before.target_far);
    assert_eq!(report.recalibration_record_hash.len(), 64);
    json!({
        "status": report.status,
        "trigger_reasons": report.trigger_reasons,
        "previous_calibration_version": report.previous_calibration_version,
        "new_calibration_version": report.new_calibration_version,
        "calibration_brier_improvement": report.calibration_brier_improvement,
        "admission_before": report.admission_before,
        "admission_after": report.admission_after,
        "recalibration_record_hash": report.recalibration_record_hash
    })
}

fn edge_empty_history_fails_loud(root: &Path) -> Value {
    let request = request_with_sources(root, "edge-empty", Vec::new(), thresholds());
    let err = run_drift_recalibration_report(&request, &root.join("edge-empty"))
        .expect_err("empty drift history rejected");
    assert_eq!(err.code(), ERR_DRIFT_RECALIBRATION_INVALID_REQUEST);
    json!({"code": err.code(), "message": err.message()})
}

fn edge_malformed_window_fails_loud(root: &Path) -> Value {
    let mut window = drift_window(50);
    window.window_end_millis = window.window_start_millis;
    let request = request_with_sources(root, "edge-malformed", vec![window], thresholds());
    let err = run_drift_recalibration_report(&request, &root.join("edge-malformed"))
        .expect_err("malformed drift window rejected");
    assert_eq!(err.code(), ERR_DRIFT_RECALIBRATION_INVALID_REQUEST);
    json!({"code": err.code(), "message": err.message()})
}

fn edge_insufficient_anchors_fails_loud(root: &Path) -> Value {
    let request = request_with_sources(root, "edge-anchors", vec![drift_window(10)], thresholds());
    let err = run_drift_recalibration_report(&request, &root.join("edge-anchors"))
        .expect_err("under-anchored drift window rejected");
    assert_eq!(err.code(), ERR_DRIFT_RECALIBRATION_INSUFFICIENT_ANCHORS);
    json!({"code": err.code(), "message": err.message()})
}

fn run_and_read(
    root: &Path,
    dir: &str,
    request: &DriftRecalibrationRequest,
) -> DriftRecalibrationReport {
    let run =
        run_drift_recalibration_report(request, &root.join(dir)).expect("drift recalibration run");
    let readback = read_drift_recalibration_report(&run.report_path).expect("read drift report");
    assert_eq!(readback, run.report);
    readback
}

fn request_with_sources(
    root: &Path,
    dir: &str,
    windows: Vec<DriftMetricWindow>,
    thresholds: DriftRecalibrationThresholds,
) -> DriftRecalibrationRequest {
    let case_dir = root.join(dir);
    let (calibration_artifact, calibration_refit) = calibration_refit(&case_dir);
    let metrics_artifact = case_dir.join("drift_metrics.json");
    write_json(
        &metrics_artifact,
        &serde_json::to_value(&windows).expect("metrics json"),
    );
    let metrics_readback: Vec<DriftMetricWindow> =
        serde_json::from_slice(&std::fs::read(&metrics_artifact).expect("read metrics"))
            .expect("decode metrics");
    assert_eq!(metrics_readback, windows);

    let admission_config_artifact = case_dir.join("admission_config.json");
    let admission_before = admission_before();
    write_json(
        &admission_config_artifact,
        &serde_json::to_value(admission_before).expect("admission json"),
    );
    DriftRecalibrationRequest {
        domain: "crypto".to_string(),
        horizon_bucket: "1h_24h".to_string(),
        metrics_artifact: metrics_artifact.display().to_string(),
        calibration_artifact: calibration_artifact.display().to_string(),
        admission_config_artifact: admission_config_artifact.display().to_string(),
        previous_calibration_version: "crypto:1h_24h:previous".to_string(),
        calibration_refit,
        admission_before,
        thresholds,
        windows,
    }
}

fn calibration_refit(case_dir: &Path) -> (PathBuf, CalibrationRefitReport) {
    let calibration_dir = case_dir.join("calibration");
    let run = run_calibration_refit(&CalibrationRefitRequest {
        out_dir: &calibration_dir,
        domain: "crypto",
        horizon_bucket: "1h_24h",
        previous_version: Some("crypto:1h_24h:previous"),
        as_of_millis: AS_OF,
        observations: calibration_observations(),
    })
    .expect("calibration refit");
    let readback = read_calibration_refit_report(&run.report_path).expect("read calibration");
    assert_eq!(readback, run.report);
    (run.report_path, readback)
}

fn drift_window(anchor_count: usize) -> DriftMetricWindow {
    DriftMetricWindow {
        window_id: "issue107-latest".to_string(),
        window_start_millis: AS_OF - 60_000,
        window_end_millis: AS_OF,
        input_mmd_p_value: 0.03125,
        association_recall_ratio: 0.875,
        calibration_abs_error: 0.1875,
        mean_brier: 0.3125,
        source_drift_score: 0.75,
        anchor_count,
    }
}

fn thresholds() -> DriftRecalibrationThresholds {
    DriftRecalibrationThresholds {
        max_mmd_p_value: 0.0625,
        min_association_recall_ratio: 0.9375,
        max_calibration_abs_error: 0.125,
        max_mean_brier: 0.25,
        max_source_drift_score: 0.5,
        min_anchor_count: 30,
    }
}

fn admission_before() -> AdmissionConfigSnapshot {
    AdmissionConfigSnapshot {
        min_p_win: 0.875,
        target_far: 0.125,
        alpha: 0.0625,
        min_grounding_anchors: 30,
        max_daily_error_score: 8.0,
    }
}

fn calibration_observations() -> Vec<CalibrationRefitObservation> {
    let mut rows = Vec::new();
    for i in 0..15 {
        rows.push(obs(0.60, i % 5 != 0, i));
    }
    for i in 0..15 {
        rows.push(obs(0.40, i % 5 == 0, 15 + i));
    }
    rows
}

fn obs(p_raw: f64, outcome_yes: bool, offset: u64) -> CalibrationRefitObservation {
    CalibrationRefitObservation {
        p_raw,
        outcome_yes,
        resolved_at_millis: AS_OF - 30_000 + offset,
    }
}
