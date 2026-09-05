use std::fs;
use std::path::Path;

use calyx_poly::forecast_calibration::{
    ERR_CAL_PROBABILITY, ERR_CAL_SAMPLES, ERR_CAL_SINGLE_CLASS,
};
use calyx_poly::{
    CALIBRATION_REFIT_REPORT_FILE, CalibrationRefitObservation, CalibrationRefitReport,
    CalibrationRefitRequest, ERR_CALIBRATION_REFIT_FUTURE_OBSERVATION,
    read_calibration_refit_report, run_calibration_refit,
};
use serde_json::{Value, json};

// calyx-shared-module: path=fsv_support.rs alias=__calyx_shared_fsv_support_rs local=support visibility=private
use crate::__calyx_shared_fsv_support_rs as support;
use support::{collect_files, hex, named_fsv_root, reset_dir, write_blake3sums, write_json};

const AS_OF: u64 = 1_785_400_000_000;
const HEALTHY_CODE: &str = "CALYX_POLY_CALIBRATION_REFIT_WRITTEN";

#[derive(Clone, Copy)]
enum CaseKind {
    Happy,
    Insufficient,
    SingleClass,
    NonFinite,
    Future,
}

#[test]
fn issue111_calibration_refit_fsv() {
    let (root, keep_root) =
        named_fsv_root("POLY_ISSUE111_FSV_ROOT", "poly-issue111-calibration-refit");
    reset_dir(&root);

    let happy = run_case(&root, "happy", CaseKind::Happy, HEALTHY_CODE);
    let insufficient = run_case(
        &root,
        "insufficient",
        CaseKind::Insufficient,
        ERR_CAL_SAMPLES,
    );
    let single_class = run_case(
        &root,
        "single-class",
        CaseKind::SingleClass,
        ERR_CAL_SINGLE_CLASS,
    );
    let nonfinite = run_case(
        &root,
        "non-finite",
        CaseKind::NonFinite,
        ERR_CAL_PROBABILITY,
    );
    let future = run_case(
        &root,
        "future-observation",
        CaseKind::Future,
        ERR_CALIBRATION_REFIT_FUTURE_OBSERVATION,
    );

    for case in [&happy, &insufficient, &single_class, &nonfinite, &future] {
        assert_eq!(case["ok"], true, "case did not meet expected proof: {case}");
    }

    let mut files = Vec::new();
    collect_files(&root, &mut files);
    let summary = json!({
        "issue": 111,
        "proof_claim": "Poly continuously re-fits and versions domain x horizon calibration slopes from resolved outcomes, persists the versioned slope report, reads it back, and fails closed on insufficient, single-class, non-finite, or future-dated histories.",
        "minimum_sufficient_proof_corpus": {
            "selected": "exactly 30 resolved observations, the existing calibration floor, split across both outcome classes, plus four one-purpose edge corpora",
            "why_smaller_would_not_prove": "the production fitter refuses fewer than 30 samples; both outcome classes are required to identify a slope",
            "why_larger_would_be_wasteful": "more rows would repeat the same deterministic fit, version hash, persistence, and readback paths without adding proof for #111"
        },
        "source_of_truth": [
            "calibration_refit_report.json read back from disk",
            "per-case before.json and after.json file-state readbacks"
        ],
        "happy_path": happy,
        "edge_cases": {
            "insufficient": insufficient,
            "single_class": single_class,
            "non_finite": nonfinite,
            "future_observation": future
        },
        "physical_files": files
    });
    write_json(&root.join("readback.json"), &summary);
    write_blake3sums(&root);
    println!(
        "issue111_fsv_summary={}",
        serde_json::to_string_pretty(&summary).expect("encode summary")
    );
    if keep_root {
        println!("poly_issue111_fsv_root={}", root.display());
    }
}

fn run_case(root: &Path, name: &str, kind: CaseKind, expected_code: &str) -> Value {
    let case_dir = root.join(name);
    reset_dir(&case_dir);
    let report_path = case_dir.join(CALIBRATION_REFIT_REPORT_FILE);
    let before = state(&report_path);
    write_json(&case_dir.join("before.json"), &before);

    let request = CalibrationRefitRequest {
        out_dir: &case_dir,
        domain: "crypto",
        horizon_bucket: "1h_24h",
        previous_version: Some("crypto:1h_24h:previous"),
        as_of_millis: AS_OF,
        observations: observations(kind),
    };
    let observed = match run_calibration_refit(&request) {
        Ok(run) => {
            let readback = read_calibration_refit_report(&run.report_path).expect("read report");
            assert_eq!(readback, run.report, "report must round-trip exactly");
            assert_eq!(readback.observation_count, 30);
            assert!(readback.brier_improvement > 0.0);
            HEALTHY_CODE.to_string()
        }
        Err(err) => err.code().to_string(),
    };

    let after = state(&report_path);
    write_json(&case_dir.join("after.json"), &after);
    let outcome = json!({
        "case": name,
        "expected_code": expected_code,
        "observed_code": observed,
        "ok": observed == expected_code,
        "before": before,
        "after": after
    });
    write_json(&case_dir.join("outcome.json"), &outcome);
    outcome
}

fn observations(kind: CaseKind) -> Vec<CalibrationRefitObservation> {
    match kind {
        CaseKind::Insufficient => happy_observations().into_iter().take(29).collect(),
        CaseKind::SingleClass => (0..30).map(|i| obs(0.60, true, i)).collect(),
        CaseKind::NonFinite => {
            let mut rows = happy_observations();
            rows[0].p_raw = f64::NAN;
            rows
        }
        CaseKind::Future => {
            let mut rows = happy_observations();
            rows[0].resolved_at_millis = AS_OF + 1;
            rows
        }
        CaseKind::Happy => happy_observations(),
    }
}

fn happy_observations() -> Vec<CalibrationRefitObservation> {
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

fn state(report_path: &Path) -> Value {
    let bytes = fs::read(report_path).ok();
    let readback = bytes
        .as_ref()
        .and_then(|b| serde_json::from_slice::<CalibrationRefitReport>(b).ok());
    json!({
        "path": report_path.display().to_string(),
        "exists": report_path.exists(),
        "bytes": bytes.as_ref().map(Vec::len).unwrap_or(0),
        "blake3": bytes.as_ref().map(|b| hex(blake3::hash(b).as_bytes())),
        "readback": readback.as_ref().map(report_summary)
    })
}

fn report_summary(report: &CalibrationRefitReport) -> Value {
    json!({
        "version": &report.version,
        "previous_version": &report.previous_version,
        "observation_count": report.observation_count,
        "positives": report.positives,
        "brier_raw": report.slope.brier_raw,
        "brier_calibrated": report.slope.brier_calibrated,
        "brier_improvement": report.brier_improvement,
        "slope_b": report.slope.b
    })
}
