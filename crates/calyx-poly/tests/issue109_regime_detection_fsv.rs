//! Issue #109 - regime detection plus per-regime parameter sets.
//!
//! Source of truth: persisted stream-source JSON and regime-detection JSON artifacts, each read back
//! from disk before assertions are recorded.

use std::path::Path;

use calyx_assay::{CusumConfig, MmdConfig, TrustTag};
use calyx_poly::regime_detection::{
    ERR_REGIME_DETECTION_INVALID_REQUEST, ERR_REGIME_DETECTION_PROVISIONAL_SOURCE, MarketRegime,
    RegimeDetectionReport, RegimeDetectionRequest, RegimeObservation, read_regime_detection_report,
    run_regime_detection_report,
};
use serde_json::{Value, json};

#[path = "fsv_support.rs"]
mod support;
use support::{collect_files, named_fsv_root, reset_dir, write_blake3sums, write_json};

const MIN_REGIME_ROWS: usize = 8;
const MMD_MIN_WINDOW: usize = 4;

#[test]
fn issue109_regime_detection_fsv() {
    let (root, _keep) = named_fsv_root("POLY_ISSUE109_FSV_ROOT", "poly-issue109-regime");
    reset_dir(&root);

    let happy = happy_compound_shift_activates_strict_parameters(&root);
    let stable = edge_stable_stream_uses_baseline_parameters(&root);
    let insufficient = edge_insufficient_rows_fail_loud(&root);
    let dimension = edge_dimension_mismatch_fails_loud(&root);
    let unsorted = edge_unsorted_timestamps_fail_loud(&root);
    let provisional = edge_provisional_source_fails_loud(&root);

    let mut files = Vec::new();
    collect_files(&root, &mut files);
    let readback = json!({
        "issue": 109,
        "proof_claim": "Poly composes Assay MMD and CUSUM detectors, labels the active market regime, persists the active and complete per-regime forecast-quality parameter sets, and refuses incomplete, malformed, unsorted, or provisional source streams.",
        "minimum_sufficient_corpus": {
            "rows": MIN_REGIME_ROWS,
            "mmd_min_window": MMD_MIN_WINDOW,
            "feature_dimensions": 2,
            "why_this_is_sufficient": "Eight rows are exactly the smallest stream that permits a 4-row pre/post MMD split while also providing enough ordered timestamps for CUSUM to see the planted event-rate shift.",
            "why_smaller_is_insufficient": "Seven rows cannot satisfy the 4+4 MMD split, so they cannot prove distribution-shift detection and regime-conditioned parameter activation together.",
            "why_larger_is_wasteful": "More rows would repeat the same Assay MMD, Assay CUSUM, report persistence, readback, and parameter-selection paths without adding a new #109 behavior."
        },
        "happy_path": happy,
        "edge_cases": {
            "stable_stream": stable,
            "insufficient_rows": insufficient,
            "dimension_mismatch": dimension,
            "unsorted_timestamps": unsorted,
            "provisional_source": provisional
        },
        "physical_files": files
    });
    let readback_path = root.join("readback.json");
    write_json(&readback_path, &readback);
    write_blake3sums(&root);
    println!(
        "ISSUE109_REGIME_DETECTION_READBACK={}",
        readback_path.display()
    );
}

fn happy_compound_shift_activates_strict_parameters(root: &Path) -> Value {
    let request = request_with_source(root, "happy", compound_observations(), TrustTag::Trusted);
    let report = run_and_read(root, "happy", &request);
    assert_eq!(report.active_regime, MarketRegime::CompoundShift);
    assert_eq!(report.active_change_index, Some(4));
    assert!(report.mmd_change_point.report.significant);
    assert!(report.cusum_change_point.change_point.is_some());
    assert_eq!(report.parameter_sets.len(), 4);
    assert_eq!(report.active_parameters.min_p_win, 0.96);
    assert_eq!(report.active_parameters.knn_k, 5);
    json!({
        "active_regime": report.active_regime,
        "active_strategy": report.active_strategy,
        "active_change_index": report.active_change_index,
        "mmd_split": report.mmd_change_point.split_index,
        "mmd_p_value": report.mmd_change_point.report.p_value,
        "cusum_change": report.cusum_change_point.change_point,
        "active_parameters": report.active_parameters,
        "detection_hash": report.detection_hash
    })
}

fn edge_stable_stream_uses_baseline_parameters(root: &Path) -> Value {
    let request = request_with_source(
        root,
        "edge-stable",
        stable_observations(),
        TrustTag::Trusted,
    );
    let report = run_and_read(root, "edge-stable", &request);
    assert_eq!(report.active_regime, MarketRegime::Stable);
    assert_eq!(report.active_change_index, None);
    assert!(!report.mmd_change_point.report.significant);
    assert!(report.cusum_change_point.change_point.is_none());
    assert_eq!(report.active_parameters.min_p_win, 0.90);
    json!({
        "active_regime": report.active_regime,
        "mmd_p_value": report.mmd_change_point.report.p_value,
        "cusum_change": report.cusum_change_point.change_point,
        "active_parameters": report.active_parameters
    })
}

fn edge_insufficient_rows_fail_loud(root: &Path) -> Value {
    let request = request_with_source(
        root,
        "edge-insufficient",
        compound_observations()[..7].to_vec(),
        TrustTag::Trusted,
    );
    let err = run_regime_detection_report(&request, &root.join("edge-insufficient"))
        .expect_err("insufficient stream rejected");
    assert_eq!(err.code(), ERR_REGIME_DETECTION_INVALID_REQUEST);
    json!({"code": err.code(), "message": err.message()})
}

fn edge_dimension_mismatch_fails_loud(root: &Path) -> Value {
    let mut rows = compound_observations();
    rows[3].features.push(9.0);
    let request = request_with_source(root, "edge-dimension", rows, TrustTag::Trusted);
    let err = run_regime_detection_report(&request, &root.join("edge-dimension"))
        .expect_err("dimension mismatch rejected");
    assert_eq!(err.code(), ERR_REGIME_DETECTION_INVALID_REQUEST);
    json!({"code": err.code(), "message": err.message()})
}

fn edge_unsorted_timestamps_fail_loud(root: &Path) -> Value {
    let mut rows = compound_observations();
    rows[5].observed_at = rows[4].observed_at;
    let request = request_with_source(root, "edge-unsorted", rows, TrustTag::Trusted);
    let err = run_regime_detection_report(&request, &root.join("edge-unsorted"))
        .expect_err("unsorted timestamps rejected");
    assert_eq!(err.code(), ERR_REGIME_DETECTION_INVALID_REQUEST);
    json!({"code": err.code(), "message": err.message()})
}

fn edge_provisional_source_fails_loud(root: &Path) -> Value {
    let request = request_with_source(
        root,
        "edge-provisional",
        compound_observations(),
        TrustTag::Provisional,
    );
    let err = run_regime_detection_report(&request, &root.join("edge-provisional"))
        .expect_err("provisional source rejected");
    assert_eq!(err.code(), ERR_REGIME_DETECTION_PROVISIONAL_SOURCE);
    json!({"code": err.code(), "message": err.message()})
}

fn run_and_read(root: &Path, dir: &str, request: &RegimeDetectionRequest) -> RegimeDetectionReport {
    let run = run_regime_detection_report(request, &root.join(dir)).expect("regime detection run");
    let readback = read_regime_detection_report(&run.report_path).expect("read regime report");
    assert_eq!(readback, run.report);
    readback
}

fn request_with_source(
    root: &Path,
    dir: &str,
    observations: Vec<RegimeObservation>,
    source_trust: TrustTag,
) -> RegimeDetectionRequest {
    let source_path = root.join(dir).join("regime_source.json");
    let request = RegimeDetectionRequest {
        domain: "crypto".to_string(),
        horizon_bucket: "1h_24h".to_string(),
        source_artifact: source_path.display().to_string(),
        source_trust,
        observations,
        mmd_min_window: MMD_MIN_WINDOW,
        mmd_config: MmdConfig {
            bandwidth: Some(1.0),
            permutations: 19,
            seed: 109,
            alpha: 0.10,
        },
        cusum_config: CusumConfig {
            baseline_gaps: 3,
            slack_k: 0.5,
            threshold_h: 2.0,
            min_sigma_frac: 0.01,
        },
    };
    write_json(
        &source_path,
        &serde_json::to_value(&request).expect("regime source json"),
    );
    let bytes = std::fs::read(&source_path).expect("read regime source");
    let readback: RegimeDetectionRequest =
        serde_json::from_slice(&bytes).expect("decode regime source");
    assert_eq!(readback, request);
    request
}

fn compound_observations() -> Vec<RegimeObservation> {
    let times = [0.0, 10.0, 20.0, 30.0, 40.0, 41.0, 42.0, 43.0];
    let features = [
        [0.0, 0.0],
        [0.1, 0.0],
        [0.0, 0.1],
        [0.1, 0.1],
        [5.0, 5.0],
        [5.1, 5.0],
        [5.0, 5.1],
        [5.1, 5.1],
    ];
    rows(&times, &features)
}

fn stable_observations() -> Vec<RegimeObservation> {
    let times = [0.0, 10.0, 20.0, 30.0, 40.0, 50.0, 60.0, 70.0];
    let features = [
        [0.0, 0.0],
        [0.1, 0.0],
        [0.0, 0.1],
        [0.1, 0.1],
        [0.0, 0.0],
        [0.1, 0.0],
        [0.0, 0.1],
        [0.1, 0.1],
    ];
    rows(&times, &features)
}

fn rows(
    times: &[f64; MIN_REGIME_ROWS],
    features: &[[f64; 2]; MIN_REGIME_ROWS],
) -> Vec<RegimeObservation> {
    times
        .iter()
        .zip(features)
        .map(|(observed_at, features)| RegimeObservation {
            observed_at: *observed_at,
            features: features.to_vec(),
        })
        .collect()
}
