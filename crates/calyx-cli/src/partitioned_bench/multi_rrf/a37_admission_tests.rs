use std::collections::BTreeMap;
use std::path::PathBuf;

use super::super::{Plan, PlanSlot};
use super::*;
use crate::assay_multi_anchor_card::model::{LensEvidence, TargetLensValue, TargetSummary};

const NAMES: [&str; 10] = [
    "lens_00", "lens_01", "lens_02", "lens_03", "lens_04", "lens_05", "lens_06", "lens_07",
    "lens_08", "lens_09",
];

#[test]
fn accepts_gate_passed_exact_roster() {
    validate(&passing_card(&NAMES), &plan(&NAMES)).unwrap();
}

#[test]
fn loads_gate_passed_card_from_aster_graph_cf() {
    let root = temp_root("a37-admission-db");
    let card = passing_card(&NAMES);
    let report = report_from_card(&card);
    crate::a37_admission_store::write(&root, "unit-gate", &report).unwrap();

    let readback = load_from_cf(Some(&root), "unit-gate", &plan(&NAMES))
        .unwrap()
        .unwrap();

    assert_eq!(readback["mode"], "assay_multi_anchor_a37_admission_db");
    assert_eq!(readback["gate_passed"], true);
    assert_eq!(readback["lens_count"], 10);
    assert_eq!(readback["db_readback"]["readback_matches"], true);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn rejects_stale_roster() {
    let mut card = passing_card(&NAMES);
    card.lenses[2].name = "other_lens".to_string();

    let err = validate(&card, &plan(&NAMES)).unwrap_err();

    assert_eq!(err.code(), "CALYX_FSV_A37_ADMISSION_CARD_STALE");
    assert!(err.message().contains("A37 admission card roster"));
}

#[test]
fn rejects_refused_gate() {
    let mut card = passing_card(&NAMES);
    card.status = "diagnostic_only".to_string();
    card.gate_passed = false;
    card.no_collapse_pass = false;

    let err = validate(&card, &plan(&NAMES)).unwrap_err();

    assert_eq!(err.code(), "CALYX_FSV_A37_ADMISSION_CARD_REFUSED");
    assert!(err.message().contains("no_collapse=false"));
}

#[test]
fn rejects_lowered_marginal_floor() {
    let mut card = passing_card(&NAMES);
    card.min_marginal_bits = 0.01;

    let err = validate(&card, &plan(&NAMES)).unwrap_err();

    assert_eq!(err.code(), "CALYX_FSV_A37_ADMISSION_CARD_REFUSED");
    assert!(err.message().contains("below required"));
}

#[test]
fn rejects_passed_lens_below_declared_floor() {
    let mut card = passing_card(&NAMES);
    card.lenses[4].best_marginal_bits = 0.04;
    card.min_best_marginal_bits = 0.04;

    let err = validate(&card, &plan(&NAMES)).unwrap_err();

    assert_eq!(err.code(), "CALYX_FSV_A37_ADMISSION_CARD_REFUSED");
    assert!(err.message().contains("below card min_marginal_bits"));
}

fn plan(names: &[&str]) -> Plan {
    Plan {
        timeline: None,
        slots: names
            .iter()
            .enumerate()
            .map(|(idx, name)| PlanSlot {
                slot: idx as u16,
                name: Some((*name).to_string()),
                lens_id: None,
                weights_sha256: None,
                signal_kind: None,
                bits_about: Some(0.1),
                vault: path(idx, "vault"),
                queries: path(idx, "queries.fbin"),
                query_start_row: 0,
                corpus: path(idx, "corpus.fbin"),
            })
            .collect(),
    }
}

fn passing_card(names: &[&str]) -> MultiAnchorAdmission {
    MultiAnchorAdmission {
        schema_version: 1,
        role: ROLE.to_string(),
        status: A37_DIVERSITY_GATE_PASSED.to_string(),
        gate_passed: true,
        lens_count: names.len(),
        passing_lens_count: names.len(),
        min_marginal_bits: 0.05,
        family_span_pass: true,
        redundancy_bound_pass: true,
        no_collapse_pass: true,
        association_family_count: 4,
        min_best_marginal_bits: 0.06,
        max_best_marginal_bits: 0.21,
        weakest_lens: names[0].to_string(),
        lenses: names
            .iter()
            .enumerate()
            .map(|(idx, name)| AdmissionLens {
                slot: idx as u16,
                name: (*name).to_string(),
                passed: true,
                best_marginal_bits: 0.06 + idx as f32 * 0.01,
            })
            .collect(),
        source_reports: vec!["report.json".to_string()],
    }
}

fn report_from_card(card: &MultiAnchorAdmission) -> MultiAnchorReport {
    MultiAnchorReport {
        schema_version: card.schema_version,
        role: card.role.clone(),
        status: card.status.clone(),
        mode: "gate".to_string(),
        gate_passed: card.gate_passed,
        report_count: 2,
        lens_count: card.lens_count,
        passing_lens_count: card.passing_lens_count,
        min_lenses: 10,
        min_marginal_bits: card.min_marginal_bits,
        max_redundancy: 0.6,
        family_span_pass: card.family_span_pass,
        redundancy_bound_pass: card.redundancy_bound_pass,
        no_collapse_pass: card.no_collapse_pass,
        association_family_count: card.association_family_count,
        association_families: BTreeMap::new(),
        min_best_marginal_bits: card.min_best_marginal_bits,
        max_best_marginal_bits: card.max_best_marginal_bits,
        weakest_lens: card.weakest_lens.clone(),
        target_summaries: vec![TargetSummary {
            target_class: 1,
            domain: "unit".to_string(),
            report_path: "assay-cf".to_string(),
            status: A37_DIVERSITY_GATE_PASSED.to_string(),
            no_collapse_pass: true,
            family_span_pass: true,
            redundancy_bound_pass: true,
            n_eff: 8.0,
            panel_bits: 0.7,
            max_marginal_bits: 0.1,
            keep_count: 10,
            park_count: 0,
        }],
        lenses: card
            .lenses
            .iter()
            .map(|lens| LensEvidence {
                slot: lens.slot,
                name: lens.name.clone(),
                association_family: "unit".to_string(),
                passed: lens.passed,
                best_target_class: 1,
                best_domain: "unit".to_string(),
                best_marginal_bits: lens.best_marginal_bits,
                best_solo_bits: 0.2,
                target_values: vec![TargetLensValue {
                    target_class: 1,
                    domain: "unit".to_string(),
                    marginal_bits: lens.best_marginal_bits,
                    solo_bits: 0.2,
                    decision: "keep".to_string(),
                }],
            })
            .collect(),
        source_reports: card.source_reports.clone(),
    }
}

fn path(idx: usize, suffix: &str) -> PathBuf {
    PathBuf::from(format!("slot_{idx}_{suffix}"))
}

fn temp_root(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "calyx-{name}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}
