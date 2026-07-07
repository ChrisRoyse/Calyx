use std::path::Path;

use calyx_poly::admission::{
    AdmissionDecision, AdmissionInputs, AdmissionParams, evaluate_admission,
};
use serde_json::{Value, json};

#[path = "fsv_support.rs"]
mod support;
use support::{
    collect_files, hex, known_healthy_market_integrity, known_healthy_oracle_risk,
    known_healthy_oracle_risk_for, known_healthy_wash_trade, named_fsv_root, reset_dir,
    write_blake3sums, write_json,
};

#[test]
fn issue168_forecast_admission_api_fsv() {
    let (root, keep_root) = named_fsv_root("POLY_ISSUE168_FSV_ROOT", "poly-issue168-admission");
    reset_dir(&root);

    let happy = happy_path_writes_admission_artifact(&root);
    let low_probability = edge_low_probability_refuses_without_admitted_artifact(&root);
    let invalid_input = edge_invalid_forecast_input_refuses(&root);
    let invalid_state = edge_invalid_confidence_state_refuses(&root);
    let cap = edge_confidence_ceiling_refuses(&root);

    let mut files = Vec::new();
    collect_files(&root, &mut files);
    write_json(
        &root.join("summary.json"),
        &json!({
            "issue": 168,
            "source_of_truth": "physical forecast-admission JSON artifacts under the FSV root",
            "happy_path": happy,
            "edge_cases": {
                "low_probability": low_probability,
                "invalid_forecast_input": invalid_input,
                "invalid_confidence_state": invalid_state,
                "confidence_ceiling": cap
            },
            "physical_files": files
        }),
    );
    write_blake3sums(&root);

    if keep_root {
        println!("poly_issue168_fsv_root={}", root.display());
    }
}

fn happy_path_writes_admission_artifact(root: &Path) -> Value {
    let artifact = root.join("happy-admission.json");
    let before = file_state(&artifact);
    let decision = evaluate_admission(&AdmissionParams::default(), &good_inputs());
    assert!(decision.admitted, "reason: {}", decision.reason);
    assert_eq!(decision.code, "CALYX_POLY_ADMISSION_ADMITTED");
    write_json(&artifact, &decision_json(&decision));
    let after = read_json(&artifact);
    assert_eq!(after["admitted"], json!(true));
    assert_eq!(after["code"], json!("CALYX_POLY_ADMISSION_ADMITTED"));
    assert_no_legacy_keys(&after);
    json!({
        "before": before,
        "after_file": file_state(&artifact),
        "readback": after
    })
}

fn edge_low_probability_refuses_without_admitted_artifact(root: &Path) -> Value {
    let artifact = root.join("edge-low-probability-admitted.json");
    let before = file_state(&artifact);
    let mut inputs = good_inputs();
    inputs.p_win = 0.85;
    inputs.oracle_risk = known_healthy_oracle_risk_for(inputs.p_win);
    let decision = evaluate_admission(&AdmissionParams::default(), &inputs);
    assert!(!decision.admitted);
    assert_eq!(decision.code, "CALYX_POLY_ADMISSION_LOW_PROBABILITY");
    let after = file_state(&artifact);
    let evidence = edge_evidence("p_win below admission floor", decision, before, after);
    write_json(&root.join("edge-low-probability-readback.json"), &evidence);
    evidence
}

fn edge_invalid_confidence_state_refuses(root: &Path) -> Value {
    let artifact = root.join("edge-invalid-state-admitted.json");
    let before = file_state(&artifact);
    let mut inputs = good_inputs();
    inputs.daily_error_score = f64::NAN;
    let decision = evaluate_admission(&AdmissionParams::default(), &inputs);
    assert!(!decision.admitted);
    assert_eq!(decision.code, "CALYX_POLY_ADMISSION_INVALID_SCORE_STATE");
    let after = file_state(&artifact);
    let evidence = edge_evidence("non-finite daily error score", decision, before, after);
    write_json(&root.join("edge-invalid-state-readback.json"), &evidence);
    evidence
}

fn edge_invalid_forecast_input_refuses(root: &Path) -> Value {
    let artifact = root.join("edge-invalid-input-admitted.json");
    let before = file_state(&artifact);
    let mut inputs = good_inputs();
    inputs.p_win = 1.20;
    let decision = evaluate_admission(&AdmissionParams::default(), &inputs);
    assert!(!decision.admitted);
    assert_eq!(decision.code, "CALYX_POLY_ADMISSION_INVALID_FORECAST_INPUT");
    let after = file_state(&artifact);
    let evidence = edge_evidence("p_win outside [0, 1]", decision, before, after);
    write_json(&root.join("edge-invalid-input-readback.json"), &evidence);
    evidence
}

fn edge_confidence_ceiling_refuses(root: &Path) -> Value {
    // #180/#184: forecast confidence must respect the never-reaches-1 ceiling. A candidate with
    // confidence == 1.0 must be refused with no admitted artifact written.
    let artifact = root.join("edge-confidence-ceiling-admitted.json");
    let before = file_state(&artifact);
    let mut inputs = good_inputs();
    inputs.confidence = 1.0;
    let decision = evaluate_admission(&AdmissionParams::default(), &inputs);
    assert!(!decision.admitted);
    assert_eq!(decision.code, "CALYX_POLY_ADMISSION_CONFIDENCE_CEILING");
    let after = file_state(&artifact);
    let evidence = edge_evidence(
        "confidence of 1.0 exceeds the never-reaches-1 ceiling",
        decision,
        before,
        after,
    );
    write_json(
        &root.join("edge-confidence-ceiling-readback.json"),
        &evidence,
    );
    evidence
}

fn edge_evidence(trigger: &str, decision: AdmissionDecision, before: Value, after: Value) -> Value {
    let decision = decision_json(&decision);
    assert_no_legacy_keys(&decision);
    json!({
        "trigger": trigger,
        "decision": decision,
        "before": before,
        "after": after
    })
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

fn decision_json(decision: &AdmissionDecision) -> Value {
    serde_json::to_value(decision).expect("serialize admission decision")
}

fn assert_no_legacy_keys(value: &Value) {
    for key in ["authorized", "stake", "bankroll", "kelly"] {
        assert!(value.get(key).is_none(), "legacy key survived: {key}");
    }
}

fn file_state(path: &Path) -> Value {
    let bytes = std::fs::read(path).ok();
    json!({
        "path": path.display().to_string(),
        "exists": path.exists(),
        "bytes": bytes.as_ref().map(Vec::len),
        "blake3": bytes
            .as_ref()
            .map(|bytes| hex(blake3::hash(bytes).as_bytes()))
    })
}

fn read_json(path: &Path) -> Value {
    serde_json::from_slice(&std::fs::read(path).expect("read JSON source of truth"))
        .expect("decode JSON source of truth")
}
