//! Issue #92 - honesty-gate hook refuses forecasts on insufficient panel evidence.
//!
//! Source of truth: persisted CalyxNative forecast JSON artifacts and direct admission decisions
//! read back from disk.

use std::path::Path;

use calyx_assay::TrustTag;
use calyx_core::FixedClock;
use calyx_poly::admission::{
    AdmissionDecision, AdmissionInputs, AdmissionParams, evaluate_admission,
};
use calyx_poly::calyx_native::{
    CalyxNativeForecast, CalyxNativeRequest, produce_calyx_native_forecast,
    read_calyx_native_forecast, write_calyx_native_forecast,
};
use calyx_poly::forecast::{ComponentKind, ForecastComponent};
use calyx_poly::forecast_ceiling::ERR_CEILING_INPUT;
use calyx_poly::superiority::SuperiorityTiers;
use serde_json::{Value, json};

// calyx-shared-module: path=fsv_support.rs alias=__calyx_shared_fsv_support_rs local=support visibility=private
use crate::__calyx_shared_fsv_support_rs as support;
use support::{
    collect_files, known_healthy_market_integrity, known_healthy_oracle_risk,
    known_healthy_wash_trade, named_fsv_root, reset_dir, write_blake3sums, write_json,
};

#[test]
fn issue092_honesty_gate_hook_fsv() {
    let (root, _keep) = named_fsv_root("POLY_ISSUE092_FSV_ROOT", "poly-issue092-honesty-gate");
    reset_dir(&root);
    let clock = FixedClock::new(1_785_600_092);

    let happy = happy_sufficient_panel_admits(&root, &clock);
    let measured_insufficient =
        edge_measured_insufficient_refuses_even_if_tier_claims_sufficient(&root, &clock);
    let caller_false = edge_caller_insufficient_refuses_even_when_bits_sufficient(&root, &clock);
    let malformed = edge_malformed_panel_bits_fails_closed(&root, &clock);
    let direct_admission = edge_direct_admission_insufficient_panel_refuses(&root);

    let mut files = Vec::new();
    collect_files(&root, &mut files);
    let readback = json!({
        "issue": 92,
        "proof_claim": "The honesty gate refuses forecast admission when the measured panel bits are below outcome entropy, even if the caller incorrectly claims the panel_sufficient tier passed.",
        "minimum_sufficient_corpus": {
            "forecast_requests": 4,
            "direct_admission_inputs": 1,
            "why_this_is_sufficient": "One sufficient forecast proves the happy path; one measured-insufficient forecast proves the bypass is closed; one caller-false forecast proves no silent upgrade; one malformed-bits request proves fail-closed input validation.",
            "why_smaller_is_insufficient": "Omitting the measured-insufficient edge would not prove the #92 hook; omitting caller-false would not prove fail-closed tier composition; omitting malformed input would not prove bad measurements fail loud.",
            "why_larger_is_wasteful": "No historical corpus is required for this hook: the invariant is over a single forecast request's panel_bits, anchor_entropy_bits, and persisted admission verdict."
        },
        "happy_path": happy,
        "edge_cases": {
            "measured_insufficient_even_if_claimed_sufficient": measured_insufficient,
            "caller_insufficient_even_when_bits_sufficient": caller_false,
            "malformed_panel_bits": malformed,
            "direct_admission_insufficient_panel": direct_admission
        },
        "physical_files": files
    });
    let readback_path = root.join("readback.json");
    write_json(&readback_path, &readback);
    write_blake3sums(&root);
    println!("ISSUE092_HONESTY_GATE_READBACK={}", readback_path.display());
}

fn happy_sufficient_panel_admits(root: &Path, clock: &FixedClock) -> Value {
    let forecast = produce_calyx_native_forecast(&request("issue92-happy", 1.0, 1.0, true), clock)
        .expect("happy forecast");
    assert!(forecast.admissible, "reason: {}", forecast.refusal_reason);
    assert!(forecast.superiority.pass);
    let readback = persist(root, "happy", &forecast);
    assert!(readback.admissible);
    assert_no_trade_keys(&serde_json::to_value(&readback).expect("forecast JSON"));
    evidence(&readback)
}

fn edge_measured_insufficient_refuses_even_if_tier_claims_sufficient(
    root: &Path,
    clock: &FixedClock,
) -> Value {
    let forecast = produce_calyx_native_forecast(
        &request("issue92-measured-insufficient", 0.40, 1.0, true),
        clock,
    )
    .expect("insufficient forecast is produced and refused");
    assert_refused_for_sufficient_tier(&forecast);
    assert_eq!(forecast.confidence_ceiling.binding, "dpi");
    let readback = persist(root, "edge-measured-insufficient", &forecast);
    assert_refused_for_sufficient_tier(&readback);
    evidence(&readback)
}

fn edge_caller_insufficient_refuses_even_when_bits_sufficient(
    root: &Path,
    clock: &FixedClock,
) -> Value {
    let forecast =
        produce_calyx_native_forecast(&request("issue92-caller-false", 1.0, 1.0, false), clock)
            .expect("caller-false forecast is produced and refused");
    assert_refused_for_sufficient_tier(&forecast);
    let readback = persist(root, "edge-caller-false", &forecast);
    assert_refused_for_sufficient_tier(&readback);
    evidence(&readback)
}

fn edge_malformed_panel_bits_fails_closed(root: &Path, clock: &FixedClock) -> Value {
    let artifact_dir = root.join("edge-malformed-bits");
    let err =
        produce_calyx_native_forecast(&request("issue92-malformed", f64::NAN, 1.0, true), clock)
            .expect_err("non-finite panel bits must fail closed");
    assert_eq!(err.code(), ERR_CEILING_INPUT);
    assert!(
        !artifact_dir.exists(),
        "malformed request must write no forecast artifact"
    );
    json!({
        "code": err.code(),
        "message": err.message(),
        "artifact_dir_exists": artifact_dir.exists()
    })
}

fn edge_direct_admission_insufficient_panel_refuses(root: &Path) -> Value {
    let mut inputs = good_inputs();
    inputs.sufficiency_ok = false;
    let decision = evaluate_admission(&AdmissionParams::default(), &inputs);
    assert!(!decision.admitted);
    assert_eq!(decision.code, "CALYX_POLY_ADMISSION_INSUFFICIENT_PANEL");
    let path = root.join("edge-direct-admission.json");
    write_json(
        &path,
        &serde_json::to_value(&decision).expect("decision JSON"),
    );
    let readback: AdmissionDecision =
        serde_json::from_slice(&std::fs::read(&path).expect("read decision"))
            .expect("decode decision");
    assert_eq!(readback.code, decision.code);
    json!({
        "path": path.display().to_string(),
        "decision": serde_json::to_value(readback).expect("decision JSON")
    })
}

fn request(
    condition_id: &str,
    panel_bits: f64,
    anchor_entropy_bits: f64,
    caller_panel_sufficient: bool,
) -> CalyxNativeRequest {
    CalyxNativeRequest {
        domain: "crypto".to_string(),
        condition_id: condition_id.to_string(),
        token_id: format!("{condition_id}-token"),
        horizon_bucket: "1h_24h".to_string(),
        components: vec![
            component(ComponentKind::KnnBaseRate, 0.93, 0.85),
            component(ComponentKind::BitsVote, 0.94, 0.90),
        ],
        calibration: None,
        raw_confidence: 0.74,
        oracle_flakiness: 0.05,
        oracle_validity: 0.98,
        panel_bits,
        anchor_entropy_bits,
        superiority_tiers: SuperiorityTiers {
            panel_sufficient: caller_panel_sufficient,
            ..strong_tiers()
        },
        evidence: None,
    }
}

fn component(kind: ComponentKind, p: f64, reliability: f64) -> ForecastComponent {
    ForecastComponent::new(kind, p, reliability, 80, TrustTag::Trusted, "issue92")
        .expect("forecast component")
}

fn strong_tiers() -> SuperiorityTiers {
    SuperiorityTiers {
        oracle_self_consistency: 0.9,
        panel_sufficient: true,
        kernel_recall_ratio: 0.97,
        min_kernel_recall_ratio: 0.95,
        calibrated: true,
        goodhart_defended: true,
        mistake_closed: true,
    }
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

fn persist(root: &Path, dir: &str, forecast: &CalyxNativeForecast) -> CalyxNativeForecast {
    let path = write_calyx_native_forecast(&root.join(dir), forecast).expect("write forecast");
    let readback = read_calyx_native_forecast(&path).expect("read forecast");
    assert_eq!(readback.provenance_hash, forecast.provenance_hash);
    assert_eq!(readback.admissible, forecast.admissible);
    assert_eq!(readback.refusal_reason, forecast.refusal_reason);
    readback
}

fn assert_refused_for_sufficient_tier(forecast: &CalyxNativeForecast) {
    assert!(!forecast.admissible);
    assert!(!forecast.superiority.pass);
    assert!(
        forecast
            .superiority
            .failing_tiers
            .contains(&"sufficient".to_string()),
        "failing tiers: {:?}",
        forecast.superiority.failing_tiers
    );
    assert!(
        forecast.refusal_reason.contains("sufficient"),
        "refusal reason: {}",
        forecast.refusal_reason
    );
}

fn evidence(forecast: &CalyxNativeForecast) -> Value {
    json!({
        "condition_id": forecast.condition_id,
        "admissible": forecast.admissible,
        "refusal_reason": forecast.refusal_reason,
        "failing_tiers": forecast.superiority.failing_tiers,
        "dpi_ceiling": forecast.confidence_ceiling.dpi,
        "confidence_binding": forecast.confidence_ceiling.binding,
        "provenance_hash": forecast.provenance_hash,
        "trust": format!("{:?}", forecast.trust)
    })
}

fn assert_no_trade_keys(value: &Value) {
    for key in ["authorized", "stake", "bankroll", "kelly", "order"] {
        assert!(value.get(key).is_none(), "trade key survived: {key}");
    }
}
