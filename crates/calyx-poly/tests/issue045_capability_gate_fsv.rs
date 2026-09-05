//! Issue #45 - capability gate wiring, Full State Verification.
//!
//! Source of truth: the measured capability-card source JSON and the persisted Poly capability gate
//! report, both read back from disk before assertions are recorded.

use std::fs;
use std::path::Path;

use calyx_core::{
    Asymmetry, LensId, Modality, Panel, QuantPolicy, Slot, SlotId, SlotKey, SlotShape, SlotState,
};
use calyx_poly::{
    ERR_CAPABILITY_GATE_INVALID_REQUEST, POLY_CAPABILITY_MAX_PAIRWISE_CORR,
    POLY_CAPABILITY_MIN_SIGNAL_BITS, PolyCapabilityGateDecisionRow, PolyCapabilityGateMeasurement,
    PolyCapabilityGateRequest, compute_poly_capability_gate_report,
    read_poly_capability_gate_report, run_poly_capability_gate_report,
};
use calyx_registry::spec::LensHealth;
use calyx_registry::{
    CapabilityCard, CapabilityGateDecision, CapabilityGateThresholds, CapabilitySignalKind,
    CostMetrics, CoverageMetrics, MetricSource, SeparationMetrics, SpreadMetrics,
};
use serde_json::json;

// calyx-shared-module: path=fsv_support.rs alias=__calyx_shared_fsv_support_rs local=support visibility=private
use crate::__calyx_shared_fsv_support_rs as support;
use support::{collect_files, named_fsv_root, reset_dir, write_blake3sums, write_json};

#[test]
fn issue045_capability_gate_fsv() {
    let (root, _keep) = named_fsv_root("POLY_ISSUE045_FSV_ROOT", "poly-issue045-capability-gate");
    reset_dir(&root);

    let thresholds = CapabilityGateThresholds::default();
    assert_eq!(thresholds.min_signal_bits, POLY_CAPABILITY_MIN_SIGNAL_BITS);
    assert_eq!(
        thresholds.max_pairwise_corr,
        POLY_CAPABILITY_MAX_PAIRWISE_CORR
    );
    let panel = known_panel();
    let source_path = root.join("capability_measurements_source.json");
    let measurements = known_measurements(&root, &panel);
    write_json(
        &source_path,
        &serde_json::to_value(&measurements).expect("measurements json"),
    );
    let read_measurements: Vec<PolyCapabilityGateMeasurement> =
        serde_json::from_slice(&fs::read(&source_path).expect("read measurement source"))
            .expect("decode measurement source");
    assert_eq!(read_measurements, measurements);

    let request = PolyCapabilityGateRequest {
        domain: "crypto".to_string(),
        panel_id: "issue045_capability_gate".to_string(),
        panel,
        thresholds,
        measured: read_measurements,
        now: 1_785_500_045,
    };
    let run = run_poly_capability_gate_report(&request, &root).expect("capability gate run");
    let readback =
        read_poly_capability_gate_report(&run.report_path).expect("read capability gate report");
    assert_eq!(readback, run.report);

    assert_eq!(readback.evaluated_count, 5);
    assert_eq!(readback.admitted_count, 2);
    assert_eq!(readback.parked_count, 2);
    assert_eq!(readback.retired_count, 1);
    assert_eq!(readback.input_panel_version, 1);
    assert_eq!(readback.output_panel_version, 4);

    let admit = decision(&readback.decisions, 0);
    let weak = decision(&readback.decisions, 1);
    let correlated = decision(&readback.decisions, 2);
    let missing = decision(&readback.decisions, 3);
    let boundary = decision(&readback.decisions, 4);

    assert_eq!(admit.decision, CapabilityGateDecision::Admit);
    assert_eq!(admit.after_state, SlotState::Active);
    assert_eq!(weak.decision, CapabilityGateDecision::Park);
    assert_eq!(weak.after_state, SlotState::Parked);
    assert!(weak.reason.contains("below 0.0500"));
    assert_eq!(correlated.decision, CapabilityGateDecision::Retire);
    assert_eq!(correlated.after_state, SlotState::Retired);
    assert!(correlated.reason.contains("above 0.6000"));
    assert_eq!(missing.decision, CapabilityGateDecision::Park);
    assert_eq!(missing.after_state, SlotState::Parked);
    assert!(missing.reason.contains("missing grounded assay"));
    assert_eq!(boundary.decision, CapabilityGateDecision::Admit);
    assert_eq!(boundary.after_state, SlotState::Active);

    let invalid = edge_lens_slot_mismatch_fails_closed(&request);
    let mut files = Vec::new();
    collect_files(&root, &mut files);
    let issue_report = json!({
        "issue": 45,
        "proof_claim": "Poly wires Calyx Registry capability-gate decisions into panel lifecycle state: learned lenses with >=0.05 grounded bits and <=0.6 pairwise correlation stay/admit active; weak or missing signal parks; high correlation retires.",
        "minimum_sufficient_corpus": {
            "measured_lens_cards": 5,
            "happy_path": 1,
            "edge_cases": 4,
            "why_this_is_sufficient": "Five measured cards are the smallest corpus that proves every #45 decision branch and both inclusive thresholds: admit above floor, park below signal floor, retire above correlation ceiling, park missing grounded signal, and admit exactly at 0.05 bits / 0.6 correlation.",
            "why_smaller_is_insufficient": "Removing any card leaves one required lifecycle outcome or boundary unproven; replacing physical readback with in-memory assertions would not prove persisted gate state.",
            "why_larger_is_wasteful": "More measured cards would repeat the same registry gate and swap-controller lifecycle paths without adding a distinct #45 branch."
        },
        "source_of_truth": {
            "measurement_source_path": source_path.display().to_string(),
            "capability_gate_report_path": run.report_path.display().to_string()
        },
        "thresholds": readback.thresholds,
        "decision_hash": readback.decision_hash,
        "counts": {
            "evaluated": readback.evaluated_count,
            "admitted": readback.admitted_count,
            "parked": readback.parked_count,
            "retired": readback.retired_count
        },
        "decisions": readback.decisions,
        "invalid_request_edge": invalid,
        "physical_files": files
    });
    let issue_report_path = root.join("issue045_capability_gate_fsv_report.json");
    write_json(&issue_report_path, &issue_report);
    let persisted_issue_report: serde_json::Value =
        serde_json::from_slice(&fs::read(&issue_report_path).expect("read issue report"))
            .expect("decode issue report");
    assert_eq!(persisted_issue_report["issue"], json!(45));

    write_blake3sums(&root);
    println!(
        "ISSUE045_CAPABILITY_GATE_FSV={}",
        issue_report_path.display()
    );
    println!(
        "ISSUE045_CAPABILITY_GATE_REPORT={}",
        run.report_path.display()
    );
}

fn edge_lens_slot_mismatch_fails_closed(request: &PolyCapabilityGateRequest) -> serde_json::Value {
    let mut invalid = request.clone();
    invalid.measured[0].card.lens_id = LensId::from_bytes([99; 16]);
    let err = compute_poly_capability_gate_report(&invalid)
        .expect_err("slot/card lens mismatch must fail closed");
    assert_eq!(err.code(), ERR_CAPABILITY_GATE_INVALID_REQUEST);
    json!({ "code": err.code(), "message": err.message() })
}

fn decision(rows: &[PolyCapabilityGateDecisionRow], slot: u16) -> &PolyCapabilityGateDecisionRow {
    rows.iter()
        .find(|row| row.slot_id == SlotId::new(slot))
        .unwrap_or_else(|| panic!("missing decision row for slot {slot}"))
}

fn known_measurements(root: &Path, panel: &Panel) -> Vec<PolyCapabilityGateMeasurement> {
    vec![
        measurement(root, panel, 0, "happy_admit", Some(0.08), 0.20),
        measurement(root, panel, 1, "edge_weak_signal", Some(0.049), 0.20),
        measurement(root, panel, 2, "edge_high_correlation", Some(0.12), 0.61),
        measurement(root, panel, 3, "edge_missing_signal", None, 0.20),
        measurement(root, panel, 4, "edge_inclusive_boundary", Some(0.05), 0.60),
    ]
}

fn measurement(
    root: &Path,
    panel: &Panel,
    slot: u16,
    name: &str,
    signal: Option<f32>,
    corr: f32,
) -> PolyCapabilityGateMeasurement {
    let slot_id = SlotId::new(slot);
    let lens_id = panel
        .slots
        .iter()
        .find(|s| s.slot_id == slot_id)
        .expect("slot")
        .lens_id;
    let evidence_path = root.join("evidence").join(format!("{name}.json"));
    let row = PolyCapabilityGateMeasurement {
        slot_id,
        card: card(lens_id, signal),
        max_pairwise_corr: corr,
        evidence_artifact: evidence_path.display().to_string(),
    };
    write_json(
        &evidence_path,
        &json!({
            "name": name,
            "slot_id": slot,
            "lens_id": lens_id,
            "signal_bits": signal,
            "max_pairwise_corr": corr
        }),
    );
    row
}

fn card(lens_id: LensId, signal: Option<f32>) -> CapabilityCard {
    CapabilityCard {
        lens_id,
        probe_count: 50,
        signal,
        signal_source: if signal.is_some() {
            MetricSource::AssayStore
        } else {
            MetricSource::AssayPending
        },
        signal_kind: CapabilitySignalKind::LearnedEncoder,
        signal_reliability: None,
        proxy_signal: signal.unwrap_or(0.0),
        differentiation: signal,
        differentiation_source: MetricSource::AssayStore,
        proxy_differentiation: 0.5,
        spread: SpreadMetrics {
            participation_ratio: 2.0,
            normalized_participation_ratio: 0.5,
            stable_rank: 2.0,
            total_variance: 1.0,
            mean_pairwise_distance: 1.0,
        },
        separation: SeparationMetrics {
            score: 0.5,
            silhouette: 0.5,
            mean_pairwise_distance: 1.0,
            labeled_groups: 2,
            used_labels: true,
        },
        cost: CostMetrics {
            total_ms: 1.0,
            ms_per_input: 0.02,
            vram_bytes: 0,
            vram_observed: true,
            ram_bytes: 0,
            batch_ceiling: 1_000,
        },
        coverage: CoverageMetrics {
            requested: 50,
            measured: 50,
            failed: 0,
            rate: 1.0,
        },
        health: LensHealth::Loaded,
        low_spread: false,
        execution: Default::default(),
    }
}

fn known_panel() -> Panel {
    Panel {
        version: 1,
        slots: (0..5).map(known_slot).collect(),
        created_at: 1_785_500_000,
        kernel_ref: None,
        guard_ref: None,
    }
}

fn known_slot(slot: u16) -> Slot {
    let slot_id = SlotId::new(slot);
    Slot {
        slot_id,
        slot_key: SlotKey::new(slot_id, format!("candidate_{slot}")),
        lens_id: LensId::from_bytes([slot as u8 + 1; 16]),
        shape: SlotShape::Dense(4),
        modality: Modality::Structured,
        asymmetry: Asymmetry::None,
        quant: QuantPolicy::None,
        resource: Default::default(),
        axis: Some(format!("candidate_{slot}")),
        retrieval_only: false,
        excluded_from_dedup: false,
        bits_about: Default::default(),
        state: SlotState::Active,
        added_at_panel_version: 1,
    }
}
