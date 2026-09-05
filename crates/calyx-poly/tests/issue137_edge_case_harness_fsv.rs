use std::fs;
use std::path::Path;

use calyx_poly::admission::{AdmissionDecision, AdmissionInputs, AdmissionParams};
use calyx_poly::edge_audit::{EdgeCaseDriver, EdgeCaseSpec, EdgeInputClass, drive_edge_case};
use serde_json::{Value, json};

// calyx-shared-module: path=fsv_support.rs alias=__calyx_shared_fsv_support_rs local=support visibility=private
use crate::__calyx_shared_fsv_support_rs as support;
use support::{
    collect_files, hex, known_healthy_market_integrity, known_healthy_oracle_risk,
    known_healthy_wash_trade, named_fsv_root, reset_dir, write_blake3sums, write_json,
};

#[test]
fn issue137_edge_case_harness_fsv() {
    let (root, keep_root) = named_fsv_root("POLY_ISSUE137_FSV_ROOT", "poly-issue137-edge-audit");
    reset_dir(&root);

    let happy = run_case(
        &root,
        "happy",
        EdgeInputClass::HappyPath,
        "CALYX_POLY_ADMISSION_ADMITTED",
        true,
        good_inputs(),
    );
    let empty = run_case(
        &root,
        "empty-evidence",
        EdgeInputClass::EmptyInput,
        "CALYX_POLY_ADMISSION_MISSING_EVIDENCE",
        false,
        empty_evidence_inputs(),
    );
    let max = run_case(
        &root,
        "max-daily-error",
        EdgeInputClass::MaxLimit,
        "CALYX_POLY_ADMISSION_DAILY_ERROR_LIMIT",
        false,
        max_daily_error_inputs(),
    );
    let invalid = run_case(
        &root,
        "invalid-nan-probability",
        EdgeInputClass::InvalidInput,
        "CALYX_POLY_ADMISSION_INVALID_FORECAST_INPUT",
        false,
        invalid_probability_inputs(),
    );

    for outcome in [&happy, &empty, &max, &invalid] {
        assert!(
            outcome.ok,
            "{} expected {} got {}",
            outcome.name, outcome.expected_code, outcome.observed_code
        );
    }

    let mut files = Vec::new();
    collect_files(&root, &mut files);
    let summary = json!({
        "issue": 137,
        "source_of_truth": [
            "per-case before.json files read from disk",
            "per-case after.json files read from disk",
            "file-backed admission-ledger.jsonl rows",
            "edge-case-outcome.json readbacks"
        ],
        "happy_path": happy,
        "edge_cases": {
            "empty_input": empty,
            "max_limit": max,
            "invalid_input": invalid
        },
        "physical_files": files
    });
    write_json(&root.join("summary.json"), &summary);
    write_blake3sums(&root);
    println!(
        "issue137_fsv_summary={}",
        serde_json::to_string_pretty(&summary).expect("encode summary")
    );
    if keep_root {
        println!("poly_issue137_fsv_root={}", root.display());
    }
}

fn run_case(
    root: &Path,
    name: &str,
    input_class: EdgeInputClass,
    expected_code: &str,
    expect_state_change: bool,
    inputs: AdmissionInputs,
) -> calyx_poly::edge_audit::EdgeCaseOutcome {
    let case_dir = root.join(name);
    reset_dir(&case_dir);
    let ledger = case_dir.join("admission-ledger.jsonl");
    let params = AdmissionParams::default();
    drive_edge_case(
        EdgeCaseSpec {
            case_dir: &case_dir,
            name,
            input_class,
            expected_code,
            expect_state_change,
        },
        EdgeCaseDriver {
            read_before: || state(&case_dir, &ledger, &inputs),
            execute: || {
                let decision = calyx_poly::admission::evaluate_admission(&params, &inputs);
                if decision.admitted {
                    append_admission(&ledger, &decision);
                }
                decision
            },
            read_after: || state(&case_dir, &ledger, &inputs),
            decision_record: |decision: AdmissionDecision| {
                (
                    decision.code.clone(),
                    serde_json::to_value(&decision).expect("decision JSON"),
                )
            },
        },
    )
    .expect("drive edge case")
}

fn append_admission(path: &Path, decision: &AdmissionDecision) {
    let line = serde_json::to_string(&json!({
        "issue": 137,
        "code": decision.code,
        "admitted": decision.admitted,
    }))
    .expect("encode admission line");
    fs::write(path, format!("{line}\n")).expect("write admission ledger");
}

fn state(case_dir: &Path, ledger: &Path, inputs: &AdmissionInputs) -> Value {
    let bytes = fs::read(ledger).ok();
    let line_count = bytes
        .as_ref()
        .map(|bytes| String::from_utf8_lossy(bytes).lines().count())
        .unwrap_or(0);
    json!({
        "case_dir": file_state(case_dir),
        "ledger": {
            "path": ledger.display().to_string(),
            "exists": ledger.exists(),
            "bytes": bytes.as_ref().map(Vec::len).unwrap_or(0),
            "blake3": bytes.as_ref().map(|bytes| hex(blake3::hash(bytes).as_bytes())),
            "admission_rows": line_count
        },
        "input": input_state(inputs)
    })
}

fn file_state(path: &Path) -> Value {
    json!({
        "path": path.display().to_string(),
        "exists": path.exists()
    })
}

fn input_state(inputs: &AdmissionInputs) -> Value {
    json!({
        "p_win": finite_or_label(inputs.p_win),
        "confidence": finite_or_label(inputs.confidence),
        "evidence_count": inputs.evidence_count,
        "source_derived_evidence_count": inputs.source_derived_evidence_count,
        "daily_error_score": finite_or_label(inputs.daily_error_score),
        "kill_switch_active": inputs.kill_switch_active
    })
}

fn finite_or_label(value: f64) -> Value {
    if value.is_finite() {
        json!(value)
    } else {
        json!(value.to_string())
    }
}

fn good_inputs() -> AdmissionInputs {
    AdmissionInputs {
        p_win: 0.94,
        confidence: 0.74,
        sufficiency_ok: true,
        evidence_count: 4,
        source_derived_evidence_count: 4,
        stale_evidence_count: 0,
        circular_evidence_count: 0,
        super_intel_pass: true,
        guard_calibrated: true,
        grounding_anchor_count: AdmissionParams::default().min_grounding_anchors,
        guard_pass: true,
        liquidity_ok: true,
        market_integrity: known_healthy_market_integrity(),
        oracle_risk: known_healthy_oracle_risk(),
        wash_trade: known_healthy_wash_trade(),
        kill_switch_active: false,
        daily_error_score: 0.0,
    }
}

fn empty_evidence_inputs() -> AdmissionInputs {
    AdmissionInputs {
        evidence_count: 0,
        source_derived_evidence_count: 0,
        ..good_inputs()
    }
}

fn max_daily_error_inputs() -> AdmissionInputs {
    // Mistake-closure circuit breaker: recent forecast error score at/above the cap must refuse.
    AdmissionInputs {
        daily_error_score: AdmissionParams::default().max_daily_error_score,
        ..good_inputs()
    }
}

fn invalid_probability_inputs() -> AdmissionInputs {
    AdmissionInputs {
        p_win: f64::NAN,
        ..good_inputs()
    }
}
