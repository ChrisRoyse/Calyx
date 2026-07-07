//! Issue #91 - Ward conformal calibration integration FSV.
//!
//! Source of truth: persisted Ward calibration/admission report JSON read back from disk.

use std::path::Path;

use calyx_core::{FixedClock, SlotId};
use calyx_poly::admission::AdmissionInputs;
use calyx_poly::ward_calibration::{
    ERR_WARD_CALIBRATION_INSUFFICIENT_ANCHORS, ERR_WARD_CALIBRATION_MALFORMED_RESIDUAL,
    ERR_WARD_CALIBRATION_STALE, WardCalibrationReport, WardCalibrationRequest,
    WardCalibrationResidual, WardResidualClass, read_ward_calibration_report,
    run_ward_calibration_report,
};
use calyx_ward::{MIN_BAD_SCORES, SlotKind};
use serde_json::{Value, json};

#[path = "fsv_support.rs"]
mod support;
use support::{
    collect_files, known_healthy_market_integrity, known_healthy_oracle_risk,
    known_healthy_wash_trade, named_fsv_root, reset_dir, write_blake3sums, write_json,
};

const CALIBRATION_TS: u64 = 1_785_600_091;
const GUARD_ID: &str = "018f48a4-9a79-74d2-8a5c-9ad7f6b8c191";
const SLOT: SlotId = SlotId::new(9);

#[test]
fn issue091_ward_calibration_fsv() {
    let (root, _keep) = named_fsv_root("POLY_ISSUE091_FSV_ROOT", "poly-issue091-ward");
    reset_dir(&root);
    let clock = FixedClock::new(CALIBRATION_TS);

    let happy = happy_calibrated_domain_admits(&root, &clock);
    let guard_refusal = edge_calibrated_guard_refuses_ood_candidate(&root, &clock);
    let insufficient = edge_insufficient_anchors_fail_closed(&root, &clock);
    let stale = edge_stale_calibration_fails_closed(&root, &clock);
    let malformed = edge_malformed_residual_fails_closed(&root, &clock);

    let mut files = Vec::new();
    collect_files(&root, &mut files);
    let readback = json!({
        "issue": 91,
        "proof_claim": "Poly uses real calyx-ward conformal calibration metadata to set forecast admission guard_calibrated/guard_pass, persists calibration provenance plus the admission ledger, and fails closed on insufficient, stale, or malformed calibration evidence.",
        "minimum_sufficient_corpus": {
            "happy_bad_anchor_rows": MIN_BAD_SCORES,
            "happy_good_anchor_rows": 1,
            "insufficient_edge_bad_anchor_rows": MIN_BAD_SCORES - 1,
            "why_this_is_sufficient": "Ward conformal calibration requires exactly 50 known-bad scores; one known-good score is the smallest extra row that proves acceptance-side residual metadata without changing the code path.",
            "why_smaller_is_insufficient": "49 known-bad rows fail Ward's conformal floor and cannot produce calibrated tau/provenance; zero good rows would not prove good-side residual evidence is retained.",
            "why_larger_is_wasteful": "More calibration rows would repeat the same Ward calibrate, high-stakes guard, admission, persistence, and readback paths without adding proof for #91; scale is not the claim."
        },
        "happy_path": happy,
        "edge_cases": {
            "calibrated_guard_refuses_ood_candidate": guard_refusal,
            "insufficient_anchors": insufficient,
            "stale_calibration": stale,
            "malformed_residual": malformed
        },
        "physical_files": files
    });
    let readback_path = root.join("readback.json");
    write_json(&readback_path, &readback);
    write_blake3sums(&root);
    println!(
        "ISSUE091_WARD_CALIBRATION_READBACK={}",
        readback_path.display()
    );
}

fn happy_calibrated_domain_admits(root: &Path, clock: &FixedClock) -> Value {
    let run = run_ward_calibration_report(
        &request(
            "happy",
            anchored_residuals(),
            CALIBRATION_TS as i64 + 60,
            3_600,
            1.0,
        ),
        &root.join("happy"),
        clock,
    )
    .expect("happy Ward calibration");
    let readback = read(&run.report_path);
    assert_eq!(readback, run.report);
    assert!(readback.guard_calibrated);
    assert!(readback.guard_pass);
    assert!(
        readback.admission_ledger.admitted,
        "{}",
        readback.admission_ledger.reason
    );
    assert_eq!(readback.bad_anchor_count, MIN_BAD_SCORES);
    assert_eq!(readback.good_anchor_count, 1);
    assert_eq!(readback.calibration_meta.per_slot_count, 1);
    assert_no_trade_keys(&serde_json::to_value(&readback).expect("report JSON"));
    evidence(&readback)
}

fn edge_calibrated_guard_refuses_ood_candidate(root: &Path, clock: &FixedClock) -> Value {
    let run = run_ward_calibration_report(
        &request(
            "guard-refuses",
            anchored_residuals(),
            CALIBRATION_TS as i64 + 60,
            3_600,
            0.0,
        ),
        &root.join("edge-guard-refuses"),
        clock,
    )
    .expect("guard-refusal Ward calibration");
    let readback = read(&run.report_path);
    assert!(readback.guard_calibrated);
    assert!(!readback.guard_pass);
    assert!(!readback.admission_ledger.admitted);
    assert_eq!(
        readback.admission_ledger.code,
        "CALYX_POLY_ADMISSION_GUARD_REFUSED"
    );
    evidence(&readback)
}

fn edge_insufficient_anchors_fail_closed(root: &Path, clock: &FixedClock) -> Value {
    let err = run_ward_calibration_report(
        &request(
            "insufficient",
            bad_residuals(MIN_BAD_SCORES - 1),
            CALIBRATION_TS as i64 + 60,
            3_600,
            1.0,
        ),
        &root.join("edge-insufficient"),
        clock,
    )
    .expect_err("insufficient anchors must fail closed");
    assert_eq!(err.code(), ERR_WARD_CALIBRATION_INSUFFICIENT_ANCHORS);
    json!({"code": err.code(), "message": err.message()})
}

fn edge_stale_calibration_fails_closed(root: &Path, clock: &FixedClock) -> Value {
    let err = run_ward_calibration_report(
        &request(
            "stale",
            anchored_residuals(),
            CALIBRATION_TS as i64 + 11,
            10,
            1.0,
        ),
        &root.join("edge-stale"),
        clock,
    )
    .expect_err("stale calibration must fail closed");
    assert_eq!(err.code(), ERR_WARD_CALIBRATION_STALE);
    json!({"code": err.code(), "message": err.message()})
}

fn edge_malformed_residual_fails_closed(root: &Path, clock: &FixedClock) -> Value {
    let mut residuals = anchored_residuals();
    residuals[0].score = f32::NAN;
    let err = run_ward_calibration_report(
        &request(
            "malformed",
            residuals,
            CALIBRATION_TS as i64 + 60,
            3_600,
            1.0,
        ),
        &root.join("edge-malformed"),
        clock,
    )
    .expect_err("malformed residual must fail closed");
    assert_eq!(err.code(), ERR_WARD_CALIBRATION_MALFORMED_RESIDUAL);
    json!({"code": err.code(), "message": err.message()})
}

fn request(
    name: &str,
    residuals: Vec<WardCalibrationResidual>,
    now_ts: i64,
    max_age_seconds: i64,
    candidate_score: f32,
) -> WardCalibrationRequest {
    WardCalibrationRequest {
        calibration_version: format!("issue091-{name}-v1"),
        domain: "crypto".to_string(),
        horizon_bucket: "1h_24h".to_string(),
        panel_version: 91,
        guard_id: GUARD_ID.to_string(),
        slot: SLOT,
        slot_kind: SlotKind::Content,
        target_far: 0.03,
        alpha: 0.05,
        min_anchor_count: MIN_BAD_SCORES,
        max_age_seconds,
        now_ts,
        candidate_score,
        residuals,
        admission_params: Default::default(),
        admission_inputs: good_inputs(),
    }
}

fn anchored_residuals() -> Vec<WardCalibrationResidual> {
    let mut residuals = bad_residuals(MIN_BAD_SCORES);
    residuals.push(WardCalibrationResidual {
        slot: SLOT,
        class: WardResidualClass::KnownGood,
        score: 0.95,
    });
    residuals
}

fn bad_residuals(n: usize) -> Vec<WardCalibrationResidual> {
    (0..n)
        .map(|idx| WardCalibrationResidual {
            slot: SLOT,
            class: WardResidualClass::KnownBad,
            score: 0.10 + (idx as f32 * 0.001),
        })
        .collect()
}

fn good_inputs() -> AdmissionInputs {
    AdmissionInputs {
        p_win: 0.94,
        confidence: 0.74,
        sufficiency_ok: true,
        evidence_count: 2,
        source_derived_evidence_count: 2,
        stale_evidence_count: 0,
        circular_evidence_count: 0,
        super_intel_pass: true,
        guard_calibrated: false,
        grounding_anchor_count: 0,
        guard_pass: false,
        liquidity_ok: true,
        market_integrity: known_healthy_market_integrity(),
        oracle_risk: known_healthy_oracle_risk(),
        wash_trade: known_healthy_wash_trade(),
        kill_switch_active: false,
        daily_error_score: 0.0,
    }
}

fn read(path: &Path) -> WardCalibrationReport {
    read_ward_calibration_report(path).expect("read Ward calibration report")
}

fn evidence(report: &WardCalibrationReport) -> Value {
    json!({
        "calibration_version": report.admission_ledger.calibration_version,
        "domain": report.domain,
        "horizon_bucket": report.horizon_bucket,
        "anchor_count": report.anchor_count,
        "bad_anchor_count": report.bad_anchor_count,
        "good_anchor_count": report.good_anchor_count,
        "tau": report.guard_verdict.per_slot[0].tau,
        "candidate_score": report.candidate_score,
        "guard_pass": report.guard_pass,
        "calibration_confidence": report.calibration_meta.confidence,
        "residual_evidence_hash": report.residual_evidence_hash,
        "admission": report.admission_ledger,
    })
}

fn assert_no_trade_keys(value: &Value) {
    for key in ["authorized", "stake", "bankroll", "kelly", "order", "pnl"] {
        assert!(value.get(key).is_none(), "trade key survived: {key}");
    }
}
