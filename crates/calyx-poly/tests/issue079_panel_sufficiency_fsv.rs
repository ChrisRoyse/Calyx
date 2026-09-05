//! Issue #79 - persisted panel sufficiency FSV.
//!
//! Source of truth: the persisted Poly panel-sufficiency JSON artifact, including the full Assay
//! ensemble card, read back from disk.

use std::path::Path;

use calyx_assay::{EnsembleConfig, EnsembleLensInput};
use calyx_core::SlotId;
use calyx_poly::admission::{
    AdmissionDecision, AdmissionInputs, AdmissionParams, evaluate_admission,
};
use calyx_poly::panel_sufficiency::{
    PolyPanelSufficiencyReport, PolyPanelSufficiencyRequest, read_panel_sufficiency_report,
    run_panel_sufficiency_report,
};
use serde_json::{Value, json};

// calyx-shared-module: path=fsv_support.rs alias=__calyx_shared_fsv_support_rs local=support visibility=private
use crate::__calyx_shared_fsv_support_rs as support;
use support::{
    collect_files, known_healthy_market_integrity, known_healthy_oracle_risk,
    known_healthy_wash_trade, named_fsv_root, reset_dir, write_blake3sums, write_json,
};

const MIN_ASSAY_ROWS: usize = 50;
const BELOW_FLOOR_ROWS: usize = 49;
const MIN_PANEL_LENSES: usize = 3;

#[test]
fn issue079_panel_sufficiency_fsv() {
    let (root, _keep) = named_fsv_root("POLY_ISSUE079_FSV_ROOT", "poly-issue079-panel-sufficiency");
    reset_dir(&root);

    let happy = happy_sufficient_panel_persists(&root);
    let insufficient = edge_insufficient_panel_routes_deficit(&root);
    let below_floor = edge_below_sample_floor_fails_closed(&root);
    let single_class = edge_single_class_labels_fail_closed(&root);
    let nonfinite = edge_nonfinite_lens_fails_closed(&root);

    let mut files = Vec::new();
    collect_files(&root, &mut files);
    let readback = json!({
        "issue": 79,
        "proof_claim": "Poly computes panel sufficiency via the real calyx-assay ensemble card, persists the sufficiency result with per-lens deficits/proposal, and feeds admission from the readback sufficiency flag.",
        "minimum_sufficient_corpus": {
            "rows": MIN_ASSAY_ROWS,
            "lenses": MIN_PANEL_LENSES,
            "below_floor_edge_rows": BELOW_FLOOR_ROWS,
            "why_this_is_sufficient": "50 labeled rows and 3 lenses are exactly the smallest corpus that clears calyx-assay's sample floor and ensemble panel-size floor.",
            "why_smaller_is_insufficient": "49 rows fail the Assay sample floor, and fewer than 3 lenses fail the ensemble panel contract, so they cannot prove a persisted sufficiency verdict.",
            "why_larger_is_wasteful": "more rows or lenses would repeat the same Assay ensemble-card, sufficiency, deficit, write, readback, and admission paths without adding proof for #79."
        },
        "happy_path": happy,
        "edge_cases": {
            "insufficient_panel": insufficient,
            "below_sample_floor": below_floor,
            "single_class_labels": single_class,
            "nonfinite_lens": nonfinite
        },
        "physical_files": files
    });
    let readback_path = root.join("readback.json");
    write_json(&readback_path, &readback);
    write_blake3sums(&root);
    println!(
        "ISSUE079_PANEL_SUFFICIENCY_READBACK={}",
        readback_path.display()
    );
}

fn happy_sufficient_panel_persists(root: &Path) -> Value {
    let run = run(
        root,
        "happy",
        strong_request("issue79_happy", MIN_ASSAY_ROWS),
    );
    assert!(run.sufficient);
    assert_eq!(run.deficit_count, 0);
    assert!(!run.has_deficit_proposal);
    assert!(run.anchor_entropy_bits >= 0.99);
    assert!(run.panel_bits >= run.anchor_entropy_bits);
    let decision = admission_from_report(&run);
    assert!(decision.admitted, "reason: {}", decision.reason);
    json!({
        "report_path": report_path(root, "happy", &run),
        "n_samples": run.n_samples,
        "lens_count": run.lens_count,
        "anchor_entropy_bits": run.anchor_entropy_bits,
        "panel_bits": run.panel_bits,
        "sufficient": run.sufficient,
        "deficit_bits": run.deficit_bits,
        "admission": decision_json(&decision)
    })
}

fn edge_insufficient_panel_routes_deficit(root: &Path) -> Value {
    let run = run(
        root,
        "edge-insufficient",
        noise_request("issue79_noise", MIN_ASSAY_ROWS),
    );
    assert!(!run.sufficient);
    assert!(run.deficit_bits > 0.0);
    assert!(run.deficit_count > 0);
    assert!(run.has_deficit_proposal);
    let decision = admission_from_report(&run);
    assert!(!decision.admitted);
    assert_eq!(decision.code, "CALYX_POLY_ADMISSION_INSUFFICIENT_PANEL");
    json!({
        "report_path": report_path(root, "edge-insufficient", &run),
        "n_samples": run.n_samples,
        "anchor_entropy_bits": run.anchor_entropy_bits,
        "panel_bits": run.panel_bits,
        "sufficient": run.sufficient,
        "deficit_bits": run.deficit_bits,
        "deficit_count": run.deficit_count,
        "deficit_proposal": run.assay_card.deficit_proposal,
        "admission": decision_json(&decision)
    })
}

fn edge_below_sample_floor_fails_closed(root: &Path) -> Value {
    let err = run_panel_sufficiency_report(
        &strong_request("issue79_below_floor", BELOW_FLOOR_ROWS),
        &root.join("edge-below-floor"),
    )
    .expect_err("below floor rejected");
    assert_eq!(err.code(), "CALYX_ASSAY_INSUFFICIENT_SAMPLES");
    json!({"code": err.code(), "message": err.message()})
}

fn edge_single_class_labels_fail_closed(root: &Path) -> Value {
    let mut request = strong_request("issue79_single_class", MIN_ASSAY_ROWS);
    request.labels = vec![true; MIN_ASSAY_ROWS];
    let err = run_panel_sufficiency_report(&request, &root.join("edge-single-class"))
        .expect_err("single-class labels rejected");
    assert_eq!(err.code(), "CALYX_ASSAY_INSUFFICIENT_SAMPLES");
    json!({"code": err.code(), "message": err.message()})
}

fn edge_nonfinite_lens_fails_closed(root: &Path) -> Value {
    let mut request = strong_request("issue79_nonfinite", MIN_ASSAY_ROWS);
    request.lenses[0].vectors[7][0] = f32::NAN;
    let err = run_panel_sufficiency_report(&request, &root.join("edge-nonfinite"))
        .expect_err("non-finite lens rejected");
    assert_eq!(err.code(), "CALYX_ASSAY_INSUFFICIENT_SAMPLES");
    json!({"code": err.code(), "message": err.message()})
}

fn run(root: &Path, dir: &str, request: PolyPanelSufficiencyRequest) -> PolyPanelSufficiencyReport {
    let run = run_panel_sufficiency_report(&request, &root.join(dir)).expect("sufficiency run");
    let readback = read_panel_sufficiency_report(&run.report_path).expect("read report");
    assert_eq!(readback, run.report);
    readback
}

fn strong_request(panel_id: &str, n: usize) -> PolyPanelSufficiencyRequest {
    let labels = alternating_labels(n);
    request(panel_id, labels.clone(), strong_lenses(&labels))
}

fn noise_request(panel_id: &str, n: usize) -> PolyPanelSufficiencyRequest {
    let labels = alternating_labels(n);
    request(panel_id, labels.clone(), paired_noise_lenses(n))
}

fn request(
    panel_id: &str,
    labels: Vec<bool>,
    lenses: Vec<EnsembleLensInput>,
) -> PolyPanelSufficiencyRequest {
    PolyPanelSufficiencyRequest {
        domain: "crypto".to_string(),
        panel_id: panel_id.to_string(),
        panel_version: 1,
        lenses,
        labels,
        groups: None,
        config: EnsembleConfig {
            source: "issue079_fsv".to_string(),
            min_gate_lenses: MIN_PANEL_LENSES,
            min_marginal_bits: 0.05,
            max_redundancy: 0.95,
            nmi_bins: 8,
        },
    }
}

fn alternating_labels(n: usize) -> Vec<bool> {
    (0..n).map(|idx| idx % 2 == 0).collect()
}

fn strong_lenses(labels: &[bool]) -> Vec<EnsembleLensInput> {
    let mut a = Vec::with_capacity(labels.len());
    let mut b = Vec::with_capacity(labels.len());
    let mut c = Vec::with_capacity(labels.len());
    for (idx, label) in labels.iter().enumerate() {
        let signal = if *label { 1.0 } else { -1.0 };
        let jitter = ((idx % 5) as f32 - 2.0) * 0.01;
        a.push(vec![signal]);
        b.push(vec![signal * 0.8 + jitter]);
        c.push(vec![signal * 0.5 - jitter]);
    }
    vec![
        EnsembleLensInput::new("strong_a", SlotId::new(1), a),
        EnsembleLensInput::new("strong_b", SlotId::new(2), b),
        EnsembleLensInput::new("strong_c", SlotId::new(3), c),
    ]
}

fn paired_noise_lenses(n: usize) -> Vec<EnsembleLensInput> {
    let mut a = Vec::with_capacity(n);
    let mut b = Vec::with_capacity(n);
    let mut c = Vec::with_capacity(n);
    for idx in 0..n {
        let pair = idx / 2;
        a.push(vec![((pair * 17 + 3) % 11) as f32]);
        b.push(vec![((pair * 7 + 5) % 13) as f32]);
        c.push(vec![((pair * 5 + 1) % 17) as f32]);
    }
    vec![
        EnsembleLensInput::new("noise_a", SlotId::new(1), a),
        EnsembleLensInput::new("noise_b", SlotId::new(2), b),
        EnsembleLensInput::new("noise_c", SlotId::new(3), c),
    ]
}

fn admission_from_report(report: &PolyPanelSufficiencyReport) -> AdmissionDecision {
    let mut inputs = good_inputs();
    inputs.sufficiency_ok = report.sufficient;
    evaluate_admission(&AdmissionParams::default(), &inputs)
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
    serde_json::to_value(decision).expect("decision JSON")
}

fn report_path(root: &Path, dir: &str, report: &PolyPanelSufficiencyReport) -> String {
    root.join(dir)
        .join(format!(
            "panel_sufficiency_{}_{}_v{}.json",
            report.domain, report.panel_id, report.panel_version
        ))
        .display()
        .to_string()
}
