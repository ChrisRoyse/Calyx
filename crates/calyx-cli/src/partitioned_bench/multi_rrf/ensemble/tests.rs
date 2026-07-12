use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use calyx_assay::sufficiency::PanelSufficiency;
use calyx_assay::{
    ENSEMBLE_CARD_SCHEMA_VERSION, EnsembleCard, EnsembleConfig, EnsembleDecision,
    EnsembleLensValue, EnsemblePairValue, EnsembleRedundancyMethod, LINEAR_CKA_REDUNDANCY_METHOD,
    LinearCkaEstimate, PidBits, TrustTag, a37_diversity_gate,
};
use calyx_core::SlotId;

use super::super::PlanSlot;
use super::*;

#[test]
fn recall_gate_requires_ensemble_card() {
    let err = load(None, &plan(10), true).unwrap_err();
    assert_eq!(err.code(), "CALYX_FSV_A35_ENSEMBLE_CARD_REQUIRED");
}

#[test]
fn valid_ten_lens_card_reports_complete_redundancy_decomposition() {
    let root = temp_root("rrf-ensemble-card");
    let path = root.join("ensemble_card.json");
    std::fs::write(&path, serde_json::to_vec(&card(10, 0)).unwrap()).unwrap();

    let report = load(Some(&path), &plan(10), false).unwrap().unwrap();

    assert_eq!(report["panel_lens_count"], 10);
    assert_eq!(report["n_eff"], serde_json::json!(card(10, 0).n_eff));
    assert_eq!(report["card_sha256"].as_str().unwrap().len(), 64);
    assert_eq!(report["lens_values"].as_array().unwrap().len(), 10);
    assert_eq!(report["pair_values"].as_array().unwrap().len(), 45);
    assert_eq!(
        report["redundancy_method"]["metric"],
        LINEAR_CKA_REDUNDANCY_METHOD
    );
    let gate_score = report["pair_values"][0]["redundancy"]["mc_gate_upper_estimate"]
        .as_f64()
        .unwrap();
    assert!((gate_score - 0.2).abs() < 1.0e-6);
    let synergy = report["pair_values"][0]["synergy_gain_bits"]
        .as_f64()
        .unwrap();
    assert!((synergy - 0.02).abs() < 1.0e-6);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn current_card_missing_redundancy_method_fails_closed() {
    let root = temp_root("rrf-ensemble-missing-method");
    let path = root.join("ensemble_card.json");
    let mut card = card(10, 0);
    card.redundancy_method = None;
    std::fs::write(&path, serde_json::to_vec(&card).unwrap()).unwrap();

    let err = load(Some(&path), &plan(10), true).unwrap_err();

    assert_eq!(err.code(), "CALYX_FSV_A35_ENSEMBLE_CARD_INVALID");
    assert!(err.message().contains("missing redundancy method"));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn schema_downgrade_cannot_bypass_redundancy_validation() {
    let root = temp_root("rrf-ensemble-schema-downgrade");
    let path = root.join("ensemble_card.json");
    let mut card = card(10, 0);
    card.schema_version = ENSEMBLE_CARD_SCHEMA_VERSION - 1;
    card.redundancy_method = None;
    for pair in &mut card.pairs {
        pair.redundancy = None;
    }
    std::fs::write(&path, serde_json::to_vec(&card).unwrap()).unwrap();

    let err = load(Some(&path), &plan(10), true).unwrap_err();

    assert_eq!(err.code(), "CALYX_FSV_A35_ENSEMBLE_CARD_INVALID");
    assert!(err.message().contains("unsupported EnsembleCard schema"));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn current_card_missing_pair_redundancy_fails_closed() {
    let root = temp_root("rrf-ensemble-missing-pair-redundancy");
    let path = root.join("ensemble_card.json");
    let mut card = card(10, 0);
    card.pairs[0].redundancy = None;
    std::fs::write(&path, serde_json::to_vec(&card).unwrap()).unwrap();

    let err = load(Some(&path), &plan(10), true).unwrap_err();

    assert_eq!(err.code(), "CALYX_FSV_A35_ENSEMBLE_CARD_INVALID");
    assert!(err.message().contains("missing redundancy evidence"));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn current_card_tampered_redundancy_method_fails_closed() {
    let root = temp_root("rrf-ensemble-tampered-method");
    let path = root.join("ensemble_card.json");
    let mut card = card(10, 0);
    card.redundancy_method.as_mut().unwrap().tuple_plan_blake3 = "not-a-digest".to_string();
    std::fs::write(&path, serde_json::to_vec(&card).unwrap()).unwrap();

    let err = load(Some(&path), &plan(10), true).unwrap_err();

    assert_eq!(err.code(), "CALYX_FSV_A35_ENSEMBLE_CARD_INVALID");
    assert!(err.message().contains("metadata"));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn stale_card_slot_set_fails_closed() {
    let root = temp_root("rrf-ensemble-stale");
    let path = root.join("ensemble_card.json");
    std::fs::write(&path, serde_json::to_vec(&card(10, 1)).unwrap()).unwrap();
    let err = load(Some(&path), &plan(10), true).unwrap_err();
    assert_eq!(err.code(), "CALYX_FSV_A35_ENSEMBLE_CARD_STALE");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn stale_card_name_roster_fails_closed() {
    let root = temp_root("rrf-ensemble-stale-name");
    let path = root.join("ensemble_card.json");
    let mut card = card(10, 0);
    card.lenses[3].name = "wrong-lens".to_string();
    std::fs::write(&path, serde_json::to_vec(&card).unwrap()).unwrap();
    let err = load(Some(&path), &plan(10), true).unwrap_err();
    assert_eq!(err.code(), "CALYX_FSV_A35_ENSEMBLE_CARD_STALE");
    assert!(err.message().contains("slot/name roster"));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn missing_plan_name_fails_closed() {
    let root = temp_root("rrf-ensemble-missing-plan-name");
    let path = root.join("ensemble_card.json");
    std::fs::write(&path, serde_json::to_vec(&card(10, 0)).unwrap()).unwrap();
    let mut plan = plan(10);
    plan.slots[0].name = None;
    let err = load(Some(&path), &plan, true).unwrap_err();

    assert_eq!(err.code(), "CALYX_FSV_A35_PLAN_NAME_REQUIRED");
    assert!(err.message().contains("slot 0"));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn invalid_pair_slot_set_fails_closed() {
    let root = temp_root("rrf-ensemble-pair-set");
    let path = root.join("ensemble_card.json");
    let mut card = card(10, 0);
    card.pairs[0] = pair_value(1, 0);
    std::fs::write(&path, serde_json::to_vec(&card).unwrap()).unwrap();

    let err = load(Some(&path), &plan(10), true).unwrap_err();

    assert_eq!(err.code(), "CALYX_FSV_A35_ENSEMBLE_CARD_INVALID");
    assert!(err.message().contains("pair slots"));
    let _ = std::fs::remove_dir_all(root);
}

fn plan(count: u16) -> Plan {
    Plan {
        timeline: None,
        slots: (0..count).map(plan_slot).collect(),
    }
}

fn plan_slot(slot: u16) -> PlanSlot {
    PlanSlot {
        slot,
        name: Some(format!("lens-{slot}")),
        lens_id: Some(format!("{:032x}", slot + 1)),
        weights_sha256: Some(format!("{:064x}", slot + 1)),
        signal_kind: Some("learned_encoder".to_string()),
        bits_about: Some(0.1),
        vault: PathBuf::from(format!("vault-{slot}")),
        queries: PathBuf::from(format!("queries-{slot}.i8bin")),
        query_start_row: 0,
        corpus: PathBuf::from(format!("corpus-{slot}.i8bin")),
    }
}

fn card(count: u16, slot_offset: u16) -> EnsembleCard {
    let lenses = (0..count)
        .map(|idx| lens_value(idx + slot_offset))
        .collect::<Vec<_>>();
    let mut pairs = Vec::new();
    for a in 0..count {
        for b in (a + 1)..count {
            pairs.push(pair_value(a + slot_offset, b + slot_offset));
        }
    }
    let a37_diversity = a37_diversity_gate(&lenses, &pairs, &EnsembleConfig::default()).unwrap();
    EnsembleCard {
        schema_version: ENSEMBLE_CARD_SCHEMA_VERSION,
        source: "unit-test".to_string(),
        pid_method: "bounded_decision_surrogate_v1".to_string(),
        panel_lens_count: count as usize,
        n_samples: 64,
        anchor_entropy_bits: 1.0,
        panel_bits: 0.5,
        panel_ci: [0.4, 0.6],
        n_eff: a37_diversity.n_eff,
        sufficient: false,
        deficit_bits: 0.5,
        a37_diversity,
        redundancy_method: Some(redundancy_method()),
        deficit_proposal: None,
        sufficiency: PanelSufficiency {
            panel_bits: 0.5,
            sufficiency_basis_bits: 0.5,
            anchor_entropy_bits: 1.0,
            observation_scope: None,
            sufficient: false,
            deficit_bits: 0.5,
            deficits: Vec::new(),
            trust: TrustTag::Provisional,
            estimate_bound: calyx_assay::EstimateBound::LowerBound,
            power_calibration: None,
        },
        lenses,
        pairs,
        keep_count: count as usize,
        park_count: 0,
        retire_count: 0,
    }
}

fn redundancy_method() -> EnsembleRedundancyMethod {
    EnsembleRedundancyMethod {
        metric: LINEAR_CKA_REDUNDANCY_METHOD.to_string(),
        tuple_design: "blake3_counter_uniform_four_distinct_with_replacement_v1".to_string(),
        row_count: 64,
        tuple_count: 4_096,
        seed_hex: "0xca1acafe4c4b4131".to_string(),
        tuple_plan_blake3: "ab".repeat(32),
        exact: false,
        uncertainty_method: "delete_32_group_jackknife_ratio_v1".to_string(),
        uncertainty_blocks: 32,
        gate_score_method: "max_0_raw_plus_4_mc_se_clamped_1_fail_closed_v1".to_string(),
    }
}

fn lens_value(slot: u16) -> EnsembleLensValue {
    EnsembleLensValue {
        name: format!("lens-{slot}"),
        slot: SlotId::new(slot),
        role: Default::default(),
        solo_bits: 0.2,
        solo_ci: [0.1, 0.3],
        panel_without_bits: 0.45,
        marginal_bits: 0.05,
        marginal_ci: [0.02, 0.08],
        pid: PidBits {
            unique_bits: 0.05,
            redundant_bits: 0.15,
            synergistic_bits: 0.02,
        },
        max_pairwise_corr: 0.4,
        max_pairwise_nmi: 0.3,
        decision: EnsembleDecision::Keep,
        decision_reason: "unit test".to_string(),
    }
}

fn pair_value(a: u16, b: u16) -> EnsemblePairValue {
    EnsemblePairValue {
        a: format!("lens-{a}"),
        b: format!("lens-{b}"),
        slot_a: SlotId::new(a),
        slot_b: SlotId::new(b),
        corr: 0.2,
        nmi: 0.1,
        redundancy: Some(LinearCkaEstimate {
            raw_signed_point: 0.2,
            redundancy_point: 0.2,
            mc_standard_error: 0.0,
            mc_gate_upper_estimate: 0.2,
        }),
        pair_bits: 0.3,
        pair_ci: [0.2, 0.4],
        synergy_gain_bits: 0.02,
    }
}

fn temp_root(name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("{name}-{}-{nanos}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    root
}
