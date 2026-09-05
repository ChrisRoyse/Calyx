use std::path::Path;

use calyx_poly::{
    BacktestObservation, PolyError, read_backtest_report, run_backtest, write_backtest_report,
};
use serde_json::{Value, json};

// calyx-shared-module: path=fsv_support.rs alias=__calyx_shared_fsv_support_rs local=support visibility=private
use crate::__calyx_shared_fsv_support_rs as support;
use support::{collect_files, hex, named_fsv_root, reset_dir, write_blake3sums, write_json};

#[test]
fn issue142_overfit_lookahead_modeling_guards_fsv() {
    let (root, keep_root) = named_fsv_root("POLY_ISSUE142_FSV_ROOT", "poly-issue142-guards");
    reset_dir(&root);

    let happy = happy_path(&root);
    let lookahead = edge_case(
        &root,
        "edge-lookahead-feature",
        |rows| rows[0].feature_max_observed_ts = rows[0].forecast_ts + 1,
        "CALYX_POLY_BACKTEST_LOOKAHEAD_FEATURE",
    );
    let train_overlap = edge_case(
        &root,
        "edge-train-cutoff-overlap",
        |rows| rows[1].train_cutoff_ts = rows[1].forecast_ts,
        "CALYX_POLY_BACKTEST_TRAIN_CUTOFF_OVERLAP",
    );
    let redundant = edge_case(
        &root,
        "edge-redundant-row",
        |rows| rows[2].redundancy_key = rows[1].redundancy_key.clone(),
        "CALYX_POLY_BACKTEST_REDUNDANT_ROW",
    );
    let bad_fingerprint = edge_case(
        &root,
        "edge-bad-fingerprint",
        |rows| rows[3].feature_fingerprint = "not-a-64-hex-fingerprint".to_string(),
        "CALYX_POLY_BACKTEST_INVALID_FEATURE_FINGERPRINT",
    );

    let mut files = Vec::new();
    collect_files(&root, &mut files);
    write_json(
        &root.join("summary.json"),
        &json!({
            "issue": 142,
            "source_of_truth": "persisted backtest report JSON plus edge readback JSON under the FSV root",
            "happy_path": happy,
            "edge_cases": {
                "lookahead_feature": lookahead,
                "train_cutoff_overlap": train_overlap,
                "redundant_row": redundant,
                "bad_fingerprint": bad_fingerprint
            },
            "physical_files": files
        }),
    );
    write_blake3sums(&root);

    if keep_root {
        println!("poly_issue142_fsv_root={}", root.display());
    }
}

fn happy_path(root: &Path) -> Value {
    let report_path = root.join("happy").join("modeling-guard-report.json");
    let before = report_state(&report_path);
    let rows = known_truth_rows();
    let report = run_backtest(&rows).expect("known leakage-safe rows should backtest");
    write_backtest_report(&report_path, &report).expect("write report");
    let readback = read_backtest_report(&report_path).expect("read report");
    assert_eq!(readback.input_count, 6);
    assert_eq!(readback.input_fingerprint.len(), 64);
    assert!(readback.beats_market_on_evaluated_subset);
    let after = report_state(&report_path);
    let evidence = json!({
        "trigger": "six held-out rows with as-of features, non-overlapping train cutoff, unique redundancy keys, and feature fingerprints",
        "before": before,
        "after": after,
        "readback": readback
    });
    write_json(&root.join("happy-readback.json"), &evidence);
    evidence
}

fn edge_case<F>(root: &Path, name: &str, mutate: F, expected_code: &str) -> Value
where
    F: FnOnce(&mut Vec<BacktestObservation>),
{
    let report_path = root.join(name).join("modeling-guard-report.json");
    let before = report_state(&report_path);
    let mut rows = known_truth_rows();
    mutate(&mut rows);
    let error = run_backtest(&rows).expect_err("edge case must fail closed");
    let error = error_json(error);
    assert_eq!(error["code"], json!(expected_code));
    let after = report_state(&report_path);
    assert_eq!(after["present"], json!(false));
    let evidence = json!({
        "trigger": name,
        "expected_code": expected_code,
        "actual_code": error["code"],
        "before": before,
        "after": after,
        "error": error
    });
    write_json(&root.join(format!("{name}-readback.json")), &evidence);
    evidence
}

fn known_truth_rows() -> Vec<BacktestObservation> {
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
    let bytes = std::fs::read(path).ok();
    let parsed = bytes
        .as_ref()
        .and_then(|bytes| serde_json::from_slice::<Value>(bytes).ok());
    json!({
        "path": path.display().to_string(),
        "present": path.exists(),
        "bytes": bytes.as_ref().map(Vec::len),
        "blake3": bytes.as_ref().map(|bytes| hex(blake3::hash(bytes).as_bytes())),
        "json": parsed
    })
}

fn error_json(error: PolyError) -> Value {
    match error {
        PolyError::Backtest { code, message } => json!({
            "code": code,
            "message": message
        }),
        other => panic!("unexpected error variant: {other:?}"),
    }
}
