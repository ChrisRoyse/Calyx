use std::fs;
use std::path::{Path, PathBuf};

use calyx_poly::{
    BacktestObservation, PolyError, read_backtest_report, run_backtest, write_backtest_report,
};
use serde_json::{Value, json};

#[test]
fn issue134_backtest_brier_calibration_fsv() {
    let (root, keep_root) = named_fsv_root("POLY_ISSUE134_FSV_ROOT", "poly-issue134-backtest");
    reset_dir(&root);

    let happy = happy_path_report_is_persisted_and_read_back(&root);
    let empty = edge_empty_input_fails_closed(&root);
    let invalid_probability = edge_invalid_probability_fails_closed(&root);
    let not_held_out = edge_not_held_out_fails_closed(&root);
    let baseline_not_beaten = edge_baseline_not_beaten_fails_closed(&root);

    let mut files = Vec::new();
    collect_files(&root, &mut files);
    let summary = json!({
        "issue": 134,
        "source_of_truth": "persisted JSON backtest report under the FSV root",
        "happy_path": happy,
        "edge_cases": {
            "empty_input": empty,
            "invalid_probability": invalid_probability,
            "not_held_out": not_held_out,
            "baseline_not_beaten": baseline_not_beaten
        },
        "physical_files": files
    });
    write_json(&root.join("summary.json"), &summary);
    write_blake3sums(&root);

    if keep_root {
        println!("poly_issue134_fsv_root={}", root.display());
    }
}

fn happy_path_report_is_persisted_and_read_back(root: &Path) -> Value {
    let report_path = root.join("happy").join("backtest-report.json");
    let before = report_state(&report_path);
    let observations = known_truth_observations();
    let report = run_backtest(&observations).expect("known held-out data should backtest");
    write_backtest_report(&report_path, &report).expect("write report");
    let readback = read_backtest_report(&report_path).expect("read report");
    assert_report_matches(&readback, &report);

    assert_eq!(readback.input_count, 6);
    assert_eq!(readback.evaluated_count, 6);
    assert_eq!(readback.input_fingerprint.len(), 64);
    assert!(readback.beats_market_on_evaluated_subset);
    assert_close(readback.evaluated.model_brier, 0.0575);
    assert_close(readback.evaluated.market_brier, 0.22166666666666668);
    assert!(readback.evaluated.model_calibration_slope > 1.55);
    assert!(readback.evaluated.brier_improvement > 0.16);

    let after = report_state(&report_path);
    let evidence = json!({
        "trigger": "score six known held-out resolved Poly observations and persist the report",
        "before": before,
        "after": after,
        "expected": {
            "input_count": 6,
            "evaluated_count": 6,
            "model_brier": 0.0575,
            "market_brier": 0.22166666666666668,
            "beats_market_on_evaluated_subset": true
        },
        "readback": readback
    });
    write_json(&root.join("happy-readback.json"), &evidence);
    evidence
}

fn edge_empty_input_fails_closed(root: &Path) -> Value {
    edge_case(
        root,
        "edge-empty",
        Vec::new(),
        "CALYX_POLY_BACKTEST_EMPTY",
        "empty backtest input",
    )
}

fn edge_invalid_probability_fails_closed(root: &Path) -> Value {
    let mut observations = known_truth_observations();
    observations[0].p_model = 1.2;
    edge_case(
        root,
        "edge-invalid-probability",
        observations,
        "CALYX_POLY_BACKTEST_INVALID_PROBABILITY",
        "p_model outside [0, 1]",
    )
}

fn edge_not_held_out_fails_closed(root: &Path) -> Value {
    let mut observations = known_truth_observations();
    observations[2].held_out = false;
    edge_case(
        root,
        "edge-not-held-out",
        observations,
        "CALYX_POLY_BACKTEST_NOT_HELD_OUT",
        "mixed train/test row enters the backtest input",
    )
}

fn edge_baseline_not_beaten_fails_closed(root: &Path) -> Value {
    let mut observations = known_truth_observations();
    for row in &mut observations {
        std::mem::swap(&mut row.p_model, &mut row.p_market);
    }
    edge_case(
        root,
        "edge-baseline-not-beaten",
        observations,
        "CALYX_POLY_BACKTEST_BASELINE_NOT_BEATEN",
        "model Brier does not beat Polymarket aggregate on evaluated subset",
    )
}

fn edge_case(
    root: &Path,
    name: &str,
    observations: Vec<BacktestObservation>,
    expected_code: &str,
    trigger: &str,
) -> Value {
    let report_path = root.join(name).join("backtest-report.json");
    let before = report_state(&report_path);
    let err = run_backtest(&observations).expect_err("edge case must fail closed");
    let error = error_json(err);
    assert_eq!(error["code"], expected_code);
    assert!(!report_path.exists(), "edge case must not persist a report");
    let after = report_state(&report_path);
    let evidence = json!({
        "trigger": trigger,
        "before": before,
        "after": after,
        "error": error
    });
    write_json(&root.join(format!("{name}-readback.json")), &evidence);
    evidence
}

fn known_truth_observations() -> Vec<BacktestObservation> {
    [
        ("m0", "yes0", 0.10, 0.40, false),
        ("m1", "yes1", 0.20, 0.45, false),
        ("m2", "yes2", 0.35, 0.55, false),
        ("m3", "yes3", 0.65, 0.45, true),
        ("m4", "yes4", 0.80, 0.55, true),
        ("m5", "yes5", 0.90, 0.60, true),
    ]
    .into_iter()
    .enumerate()
    .map(
        |(idx, (market_id, token_id, p_model, p_market, actual_win))| BacktestObservation {
            market_id: market_id.to_string(),
            token_id: token_id.to_string(),
            held_out: true,
            train_cutoff_ts: 1_785_000_000,
            forecast_ts: 1_785_100_000 + idx as u64,
            feature_max_observed_ts: 1_785_099_900 + idx as u64,
            outcome_observed_ts: 1_785_500_000 + idx as u64,
            resolved: true,
            evaluated: true,
            redundancy_key: format!("{market_id}:{token_id}:asof"),
            feature_fingerprint: format!("{:064x}", idx + 1),
            p_model,
            p_market,
            actual_win,
        },
    )
    .collect()
}

fn report_state(path: &Path) -> Value {
    if !path.exists() {
        return json!({
            "path": path.display().to_string(),
            "present": false
        });
    }
    let bytes = fs::read(path).expect("read report state bytes");
    let json_value: Value = serde_json::from_slice(&bytes).expect("report state decodes as JSON");
    json!({
        "path": path.display().to_string(),
        "present": true,
        "bytes": bytes.len(),
        "row_hash": blake3::hash(&bytes).to_hex().to_string(),
        "json": json_value
    })
}

fn error_json(err: PolyError) -> Value {
    match err {
        PolyError::Backtest { code, message } => json!({
            "code": code,
            "message": message
        }),
        other => panic!("unexpected error variant: {other}"),
    }
}

fn write_json(path: &Path, value: &Value) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create JSON parent directory");
    }
    let bytes = serde_json::to_vec_pretty(value).expect("encode JSON evidence");
    fs::write(path, bytes).expect("write JSON evidence");
}

fn write_blake3sums(root: &Path) {
    let mut paths = Vec::new();
    collect_path_list(root, &mut paths);
    paths.sort();
    let mut lines = Vec::new();
    for path in paths {
        if path.file_name().and_then(|name| name.to_str()) == Some("BLAKE3SUMS.txt") {
            continue;
        }
        let bytes = fs::read(&path).expect("read file for BLAKE3");
        let rel = path.strip_prefix(root).expect("strip FSV root");
        lines.push(format!(
            "{}  {}",
            blake3::hash(&bytes).to_hex(),
            rel.display()
        ));
    }
    fs::write(root.join("BLAKE3SUMS.txt"), lines.join("\n")).expect("write BLAKE3SUMS");
}

fn collect_files(dir: &Path, out: &mut Vec<Value>) {
    let mut paths = Vec::new();
    collect_path_list(dir, &mut paths);
    paths.sort();
    for path in paths {
        let meta = fs::metadata(&path).expect("metadata for physical file");
        out.push(json!({
            "path": path.display().to_string(),
            "bytes": meta.len()
        }));
    }
}

fn collect_path_list(dir: &Path, out: &mut Vec<PathBuf>) {
    if !dir.exists() {
        return;
    }
    for entry in fs::read_dir(dir).expect("read FSV directory") {
        let path = entry.expect("directory entry").path();
        if path.is_dir() {
            collect_path_list(&path, out);
        } else {
            out.push(path);
        }
    }
}

fn named_fsv_root(env: &str, fallback_name: &str) -> (PathBuf, bool) {
    if let Some(value) = std::env::var_os(env) {
        return (PathBuf::from(value), true);
    }
    (
        std::env::temp_dir().join(format!("{fallback_name}-{}", std::process::id())),
        false,
    )
}

fn reset_dir(path: &Path) {
    if path.exists() {
        fs::remove_dir_all(path).expect("remove previous FSV root");
    }
    fs::create_dir_all(path).expect("create FSV root");
}

fn assert_close(actual: f64, expected: f64) {
    let delta = (actual - expected).abs();
    assert!(
        delta <= 1.0e-12,
        "actual {actual:.16} expected {expected:.16} delta {delta:.16}"
    );
}

fn assert_report_matches(
    readback: &calyx_poly::BacktestReport,
    expected: &calyx_poly::BacktestReport,
) {
    assert_eq!(readback.schema_version, expected.schema_version);
    assert_eq!(readback.source_of_truth, expected.source_of_truth);
    assert_eq!(readback.input_count, expected.input_count);
    assert_eq!(readback.held_out_count, expected.held_out_count);
    assert_eq!(readback.evaluated_count, expected.evaluated_count);
    assert_eq!(readback.input_fingerprint, expected.input_fingerprint);
    assert_eq!(
        readback.beats_market_on_evaluated_subset,
        expected.beats_market_on_evaluated_subset
    );
    assert_metrics_match(&readback.all, &expected.all);
    assert_metrics_match(&readback.evaluated, &expected.evaluated);
}

fn assert_metrics_match(
    readback: &calyx_poly::BacktestMetrics,
    expected: &calyx_poly::BacktestMetrics,
) {
    assert_eq!(readback.count, expected.count);
    assert_close(readback.model_brier, expected.model_brier);
    assert_close(readback.market_brier, expected.market_brier);
    assert_close(readback.brier_improvement, expected.brier_improvement);
    assert_close(
        readback.model_calibration_intercept,
        expected.model_calibration_intercept,
    );
    assert_close(
        readback.model_calibration_slope,
        expected.model_calibration_slope,
    );
    assert_close(
        readback.market_calibration_intercept,
        expected.market_calibration_intercept,
    );
    assert_close(
        readback.market_calibration_slope,
        expected.market_calibration_slope,
    );
}
