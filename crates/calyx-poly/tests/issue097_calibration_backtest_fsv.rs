use std::fs;
use std::path::Path;

use calyx_poly::{
    CALIBRATION_BACKTEST_REPORT_FILE, CalibrationBacktestObservation, CalibrationBacktestRequest,
    PolyError, read_calibration_backtest_report, run_calibration_backtest_report,
};
use serde_json::{Value, json};

#[path = "fsv_support.rs"]
mod support;
use support::{collect_files, named_fsv_root, reset_dir, write_blake3sums, write_json};

const AS_OF_TS: u64 = 1_786_000_000;

#[test]
fn issue097_calibration_backtest_held_out_outcomes_fsv() {
    let (root, keep_root) = named_fsv_root(
        "POLY_ISSUE097_FSV_ROOT",
        "poly-issue097-calibration-backtest",
    );
    reset_dir(&root);

    let happy = happy_path(&root);
    let empty = edge_case(
        &root,
        "edge-empty-holdout",
        |mut request| {
            request.observations.clear();
            request
        },
        "CALYX_POLY_CALIBRATION_BACKTEST_EMPTY_HOLDOUT",
    );
    let missing_anchor = edge_case(
        &root,
        "edge-missing-anchor",
        |mut request| {
            request.observations[0].anchor_id.clear();
            request
        },
        "CALYX_POLY_CALIBRATION_BACKTEST_MISSING_ANCHOR",
    );
    let future_outcome = edge_case(
        &root,
        "edge-future-outcome",
        |mut request| {
            request.observations[1].outcome_observed_ts = AS_OF_TS + 1;
            request
        },
        "CALYX_POLY_CALIBRATION_BACKTEST_FUTURE_OUTCOME",
    );
    let leakage = edge_case(
        &root,
        "edge-lookahead-feature",
        |mut request| {
            request.observations[2].feature_max_observed_ts =
                request.observations[2].forecast_ts + 1;
            request
        },
        "CALYX_POLY_CALIBRATION_BACKTEST_LEAKAGE",
    );
    let insufficient = edge_case(
        &root,
        "edge-insufficient-holdout",
        |mut request| {
            request.min_held_out_rows = 7;
            request
        },
        "CALYX_POLY_CALIBRATION_BACKTEST_INSUFFICIENT_HOLDOUT",
    );

    let mut files = Vec::new();
    collect_files(&root, &mut files);
    write_json(
        &root.join("summary.json"),
        &json!({
            "issue": 97,
            "proof_claim": "Poly backtests forecast calibration on held-out resolved outcomes by joining forecast rows to local resolved anchors, writing metrics, and reading the report back from disk.",
            "minimum_sufficient_corpus": {
                "chosen_rows": 6,
                "why_sufficient": "Six known-truth held-out rows prove Brier, exact reliability-bin counts for five bins, three domain/horizon coverage buckets, source corpus readback, and both outcome classes.",
                "why_smaller_insufficient": "Fewer rows would collapse at least one required coverage bucket or reliability-bin count and would not prove all #97 metrics and refusal paths together.",
                "why_larger_wasteful": "More rows would repeat the same anchor validation, binning, Brier, coverage, persistence, and readback paths without adding proof for #97 correctness."
            },
            "source_of_truth": "local JSON forecast-anchor corpus plus persisted calibration backtest report",
            "happy_path": happy,
            "edge_cases": {
                "empty_holdout": empty,
                "missing_anchor": missing_anchor,
                "future_outcome": future_outcome,
                "leakage": leakage,
                "insufficient_holdout": insufficient
            },
            "physical_files": files
        }),
    );
    write_blake3sums(&root);

    if keep_root {
        println!("poly_issue097_fsv_root={}", root.display());
    }
}

fn happy_path(root: &Path) -> Value {
    let out_dir = root.join("happy");
    let report_path = out_dir.join(CALIBRATION_BACKTEST_REPORT_FILE);
    let corpus_path = out_dir.join("forecast-anchor-corpus.json");
    let before = report_state(&report_path);
    let request = known_request();
    write_json(
        &corpus_path,
        &serde_json::to_value(&request).expect("request to JSON"),
    );
    let corpus_readback: CalibrationBacktestRequest =
        serde_json::from_slice(&fs::read(&corpus_path).expect("read corpus"))
            .expect("decode corpus");
    assert_eq!(corpus_readback, request);

    let run = run_calibration_backtest_report(&out_dir, &corpus_readback)
        .expect("known held-out calibration backtest should pass");
    let readback = read_calibration_backtest_report(&run.report_path).expect("read report");
    assert_eq!(readback, run.report);
    assert_eq!(readback.input_count, 6);
    assert_eq!(readback.held_out_count, 6);
    assert_eq!(readback.bin_count, 5);
    assert_close(readback.brier, 0.0575);
    assert_close(readback.calibration_abs_error, 0.21666666666666667);
    assert_close(readback.direction_accuracy, 1.0);
    let counts: Vec<_> = readback
        .reliability_bins
        .iter()
        .map(|bin| bin.count)
        .collect();
    assert_eq!(counts, vec![1, 2, 0, 1, 2]);
    assert_eq!(readback.domain_horizon_coverage.len(), 3);
    assert_eq!(readback.domain_horizon_coverage[0].count, 2);
    assert_eq!(readback.domain_horizon_coverage[1].count, 2);
    assert_eq!(readback.domain_horizon_coverage[2].count, 2);

    let after = report_state(&report_path);
    let evidence = json!({
        "trigger": "score six known-truth held-out forecast rows joined to resolved anchors",
        "before": before,
        "source_corpus_readback": corpus_readback,
        "after": after,
        "expected": {
            "rows": 6,
            "bin_counts": counts,
            "brier": 0.0575,
            "calibration_abs_error": 0.21666666666666667,
            "domain_horizon_bucket_count": 3
        },
        "readback": readback
    });
    write_json(&root.join("happy-readback.json"), &evidence);
    evidence
}

fn edge_case<F>(root: &Path, name: &str, mutate: F, expected_code: &str) -> Value
where
    F: FnOnce(CalibrationBacktestRequest) -> CalibrationBacktestRequest,
{
    let out_dir = root.join(name);
    let report_path = out_dir.join(CALIBRATION_BACKTEST_REPORT_FILE);
    let request_path = out_dir.join("forecast-anchor-corpus.json");
    let request = mutate(known_request());
    let before = report_state(&report_path);
    write_json(
        &request_path,
        &serde_json::to_value(&request).expect("request to JSON"),
    );
    let request_readback: CalibrationBacktestRequest =
        serde_json::from_slice(&fs::read(&request_path).expect("read edge request"))
            .expect("decode edge request");
    let err = run_calibration_backtest_report(&out_dir, &request_readback)
        .expect_err("edge case must fail closed");
    let error = error_json(err);
    assert_eq!(error["code"], json!(expected_code));
    let after = report_state(&report_path);
    assert_eq!(after["present"], json!(false));
    let evidence = json!({
        "trigger": name,
        "expected_code": expected_code,
        "actual_code": error["code"],
        "before": before,
        "source_corpus_readback": request_readback,
        "after": after,
        "error": error
    });
    write_json(&root.join(format!("{name}-readback.json")), &evidence);
    evidence
}

fn known_request() -> CalibrationBacktestRequest {
    CalibrationBacktestRequest {
        as_of_ts: AS_OF_TS,
        min_held_out_rows: 6,
        bin_count: 5,
        observations: [
            ("crypto", "lt_1h", 0.10, false),
            ("crypto", "lt_1h", 0.20, false),
            ("politics", "gt_7d", 0.35, false),
            ("politics", "gt_7d", 0.65, true),
            ("sports", "1d_7d", 0.80, true),
            ("sports", "1d_7d", 0.90, true),
        ]
        .into_iter()
        .enumerate()
        .map(|(idx, (domain, horizon_bucket, probability, actual_win))| {
            let forecast_ts = 1_785_100_000 + idx as u64;
            CalibrationBacktestObservation {
                forecast_id: format!("forecast-{idx}"),
                market_id: format!("market-{idx}"),
                outcome_id: format!("yes-{idx}"),
                domain: domain.to_string(),
                horizon_bucket: horizon_bucket.to_string(),
                held_out: true,
                train_cutoff_ts: forecast_ts - 10_000,
                forecast_ts,
                feature_max_observed_ts: forecast_ts - 10,
                outcome_observed_ts: 1_785_500_000 + idx as u64,
                anchor_id: format!("anchor-{idx}"),
                anchor_source: "local-resolved-outcome-anchor".to_string(),
                anchor_version: 1,
                probability,
                actual_win,
            }
        })
        .collect(),
    }
}

fn report_state(path: &Path) -> Value {
    let bytes = fs::read(path).ok();
    let parsed = bytes
        .as_ref()
        .and_then(|bytes| serde_json::from_slice::<Value>(bytes).ok());
    json!({
        "path": path.display().to_string(),
        "present": path.exists(),
        "bytes": bytes.as_ref().map(Vec::len),
        "blake3": bytes.as_ref().map(|bytes| blake3::hash(bytes).to_hex().to_string()),
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

fn assert_close(actual: f64, expected: f64) {
    let delta = (actual - expected).abs();
    assert!(
        delta <= 1.0e-12,
        "actual {actual:.16} expected {expected:.16} delta {delta:.16}"
    );
}
