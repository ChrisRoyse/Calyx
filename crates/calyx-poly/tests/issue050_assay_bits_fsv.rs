//! Issue #50 — Assay bits integration, Full State Verification.
//!
//! Source of truth: durable AsterVault Assay CF rows written by `AssayStore`,
//! reloaded from the vault, plus the persisted `assay_bits_*.json` report read
//! back separately. The fixture uses the smallest decisive corpus: 50 anchored
//! rows (the KSG sample floor), two slots (the smallest redundancy pair), one
//! signal slot, and one deterministic pseudo-noise slot.

use std::path::Path;

use calyx_assay::{AssayStore, AssaySubject, EstimateBound, TrustTag};
use calyx_aster::cf::ColumnFamily;
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{AnchorKind, AnchorValue, FixedClock, SlotId, VaultId};
use calyx_poly::assay_bits::{
    ASSAY_BITS_SCHEMA_VERSION, AssayBitsRequest, DEFAULT_ASSAY_BITS_K,
    ERR_ASSAY_BITS_DEGENERATE_OUTCOME, ERR_ASSAY_BITS_NON_BOOL_ANCHOR, SlotAssayBits,
    read_assay_bits_report, run_assay_bits_to_vault,
};
use calyx_poly::panel_diagnostics::PanelMatrix;
use serde_json::{Value, json};

#[path = "fsv_support.rs"]
mod support;
#[path = "synthetic_panels.rs"]
mod synthetic;

use support::{collect_files, named_fsv_root, reset_dir, write_blake3sums, write_json};

const PANEL_VERSION: u32 = 1;
const VAULT_SALT: &[u8] = b"issue050-assay-bits-salt";

#[test]
fn issue050_assay_bits_fsv() {
    let (root, _keep) = named_fsv_root("POLY_ISSUE050_FSV_ROOT", "poly-issue050-assay-bits");
    reset_dir(&root);
    let clock = FixedClock::new(1_785_500_000);

    let happy = happy_assay_bits_round_trip(&root, &clock);
    let below_floor = edge_error(
        &root,
        "edge-below-floor",
        matrix_with_rows(49),
        "CALYX_ASSAY_INSUFFICIENT_SAMPLES",
        &clock,
    );
    let degenerate = edge_error(
        &root,
        "edge-degenerate-outcome",
        degenerate_outcome_matrix(),
        ERR_ASSAY_BITS_DEGENERATE_OUTCOME,
        &clock,
    );
    let redundant = edge_error(
        &root,
        "edge-redundant-slots",
        redundant_matrix(),
        "CALYX_ASSAY_REDUNDANT",
        &clock,
    );
    let non_bool = edge_error(
        &root,
        "edge-non-bool-anchor",
        non_bool_anchor_matrix(),
        ERR_ASSAY_BITS_NON_BOOL_ANCHOR,
        &clock,
    );

    let mut files = Vec::new();
    collect_files(&root, &mut files);
    let summary = json!({
        "issue": 50,
        "proof_claim": "Poly measures per-slot KSG MI bits against grounded boolean outcome anchors, enforces Assay's redundancy contract before persistence, writes scoped AssayStore rows to Aster Assay CF, reloads the rows from the vault, and persists a report that matches readback.",
        "minimum_sufficient_corpus": {
            "happy_path_rows": 50,
            "happy_path_slots": 2,
            "happy_path_assay_rows": 3,
            "edge_cases": 4,
            "why_this_is_sufficient": "50 rows is the current mixed continuous-discrete KSG sample floor; two slots are the smallest panel that proves one redundancy pair and per-slot ordering; one signal and one pseudo-noise slot prove the estimator distinguishes outcome information without redundant slots.",
            "why_smaller_is_insufficient": "49 rows cannot exercise the accepted KSG path; one slot cannot prove redundancy enforcement; omitting edge fixtures would not prove fail-closed no-write behavior.",
            "why_larger_is_wasteful": "additional rows or slots repeat the same label extraction, KSG, redundancy, Assay CF write/readback, and report persistence paths without adding a distinct #50 invariant."
        },
        "source_of_truth": "AsterVault Assay column family plus persisted assay_bits report JSON",
        "happy_path": happy,
        "edge_cases": {
            "below_floor": below_floor,
            "degenerate_outcome": degenerate,
            "redundant_slots": redundant,
            "non_bool_anchor": non_bool
        },
        "physical_files": files
    });
    let summary_path = root.join("issue050_assay_bits_fsv_report.json");
    write_json(&summary_path, &summary);
    write_blake3sums(&root);
    println!("ISSUE050_ASSAY_BITS_FSV={}", summary_path.display());
}

fn happy_assay_bits_round_trip(root: &Path, clock: &FixedClock) -> Value {
    let case_dir = root.join("happy");
    let vault = open_vault(&case_dir.join("vault"), *clock);
    let matrix = matrix_with_rows(50);
    let before_count = assay_count(&vault);
    assert_eq!(before_count, 0, "fresh vault starts with empty Assay CF");

    let run = run_assay_bits_to_vault(
        &vault,
        &request("happy-corpus"),
        &matrix,
        clock,
        &case_dir.join("reports"),
    )
    .expect("happy assay bits run");
    assert_eq!(run.report.schema_version, ASSAY_BITS_SCHEMA_VERSION);
    assert_eq!(run.report.n_samples, 50);
    assert_eq!(run.report.persisted_rows, 3, "two lens rows plus entropy");
    assert_eq!(run.report.trust, TrustTag::Trusted);
    assert_eq!(
        serde_json::to_value(&run.report.slot_bits[0]).expect("slot bits JSON")["bound"],
        json!("point"),
        "the public slot projection must retain the mixed estimator's Point contract"
    );
    assert_eq!(run.report.slot_bits[0].bound, Some(EstimateBound::Point));
    let mut legacy_slot = serde_json::to_value(&run.report.slot_bits[0]).expect("legacy slot JSON");
    legacy_slot
        .as_object_mut()
        .expect("slot object")
        .remove("bound");
    let legacy_slot: SlotAssayBits =
        serde_json::from_value(legacy_slot).expect("legacy slot without bound");
    assert_eq!(legacy_slot.bound, None);
    assert_eq!(assay_count(&vault), 3);
    assert_eq!(
        read_assay_bits_report(&run.report_path).expect("read report"),
        run.report
    );

    let store = AssayStore::load_from_vault(&vault).expect("load assay rows");
    let key = run.report.assay_rows[0].cache_key.clone();
    let signal = store
        .get(
            &key,
            &AssaySubject::Lens {
                slot: SlotId::new(0),
            },
        )
        .expect("signal lens row");
    let noise = store
        .get(
            &key,
            &AssaySubject::Lens {
                slot: SlotId::new(1),
            },
        )
        .expect("noise lens row");
    let entropy = store
        .get(&key, &AssaySubject::OutcomeEntropy)
        .expect("entropy row");
    assert_eq!(signal.estimate.bound, EstimateBound::Point);
    assert_eq!(noise.estimate.bound, EstimateBound::Point);
    assert!(
        signal.estimate.bits > noise.estimate.bits,
        "known signal slot must carry more bits than pseudo-noise"
    );
    assert!(
        signal.estimate.bits >= 0.05,
        "known signal slot must clear the Assay signal floor"
    );
    assert!(run.report.redundancy_pairs[0].redundancy <= 0.6);
    assert_eq!(
        entropy.estimate.estimator,
        calyx_assay::EstimatorKind::OutcomeEntropy
    );
    assert!((run.report.outcome_entropy_bits - 1.0).abs() < 1e-6);

    write_json(
        &case_dir.join("happy_summary.json"),
        &json!({
            "report_path": run.report_path.display().to_string(),
            "persisted_seq": run.persisted_seq,
            "assay_cf_count_before": before_count,
            "assay_cf_count_after": assay_count(&vault),
            "signal_bits": signal.estimate.bits,
            "noise_bits": noise.estimate.bits,
            "outcome_entropy_bits": run.report.outcome_entropy_bits,
            "redundancy": run.report.redundancy_pairs[0].redundancy,
            "assay_row_order": run.report.assay_row_order,
            "provenance_hash": run.report.provenance_hash,
        }),
    );

    json!({
        "report_path": run.report_path.display().to_string(),
        "persisted_seq": run.persisted_seq,
        "persisted_rows": run.persisted_rows,
        "signal_bits": signal.estimate.bits,
        "noise_bits": noise.estimate.bits,
        "redundancy": run.report.redundancy_pairs[0].redundancy,
        "assay_row_order": run.report.assay_row_order,
        "trust": format!("{:?}", run.report.trust),
    })
}

fn edge_error(
    root: &Path,
    name: &str,
    matrix: PanelMatrix,
    expected_code: &str,
    clock: &FixedClock,
) -> Value {
    let case_dir = root.join(name);
    let vault = open_vault(&case_dir.join("vault"), *clock);
    let err = run_assay_bits_to_vault(
        &vault,
        &request(name),
        &matrix,
        clock,
        &case_dir.join("reports"),
    )
    .expect_err("edge case must fail closed");
    assert_eq!(err.code(), expected_code);
    assert_eq!(
        assay_count(&vault),
        0,
        "edge {name} must not write Assay CF rows"
    );
    let evidence = json!({
        "code": err.code(),
        "message": err.message(),
        "assay_cf_count_after": assay_count(&vault),
        "report_dir_exists": case_dir.join("reports").exists()
    });
    write_json(&case_dir.join("edge_error.json"), &evidence);
    evidence
}

fn matrix_with_rows(n: usize) -> PanelMatrix {
    let mut signal = Vec::with_capacity(n);
    let mut noise = Vec::with_capacity(n);
    let mut anchors = Vec::with_capacity(n);
    for i in 0..n {
        let won = i % 2 == 0;
        let signal_center = if won { 1.0 } else { -1.0 };
        signal.push(signal_center + i as f32 * 0.001);
        noise.push(((i * 37) % 101) as f32 / 50.0 - 1.0);
        anchors.push(synthetic::resolved_anchor(won, i));
    }
    PanelMatrix::new(
        vec!["known_signal".to_string(), "pseudo_noise".to_string()],
        vec![signal, noise],
        anchors,
    )
    .expect("valid panel matrix")
}

fn degenerate_outcome_matrix() -> PanelMatrix {
    let mut matrix = matrix_with_rows(50);
    let anchors = (0..50)
        .map(|i| synthetic::resolved_anchor(true, i))
        .collect();
    matrix = PanelMatrix::new(
        matrix.slot_keys().to_vec(),
        matrix.columns().to_vec(),
        anchors,
    )
    .expect("degenerate labels still form a valid matrix");
    matrix
}

fn redundant_matrix() -> PanelMatrix {
    let base = matrix_with_rows(50);
    PanelMatrix::new(
        vec!["copy_a".to_string(), "copy_b".to_string()],
        vec![base.columns()[0].clone(), base.columns()[0].clone()],
        base.anchors().to_vec(),
    )
    .expect("redundant panel matrix")
}

fn non_bool_anchor_matrix() -> PanelMatrix {
    let mut base = matrix_with_rows(50);
    let mut anchors = base.anchors().to_vec();
    anchors[0].value = AnchorValue::Number(1.0);
    base = PanelMatrix::new(base.slot_keys().to_vec(), base.columns().to_vec(), anchors)
        .expect("non-bool anchor still has aligned rows");
    base
}

fn request(corpus_shard: &str) -> AssayBitsRequest {
    AssayBitsRequest {
        domain: "crypto".to_string(),
        panel_version: PANEL_VERSION,
        corpus_shard: corpus_shard.to_string(),
        anchor_kind: AnchorKind::TestPass,
        k_neighbors: DEFAULT_ASSAY_BITS_K,
    }
}

fn open_vault(dir: &Path, clock: FixedClock) -> AsterVault<FixedClock> {
    AsterVault::new_durable_with_clock(
        dir,
        vault_id(),
        VAULT_SALT.to_vec(),
        VaultOptions::default(),
        clock,
    )
    .expect("open durable test vault")
}

fn assay_count(vault: &AsterVault<FixedClock>) -> usize {
    vault
        .scan_cf_at(vault.latest_seq(), ColumnFamily::Assay)
        .expect("scan Assay CF")
        .len()
}

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap()
}
