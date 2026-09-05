//! Issue #208 — Loom pair-gain materialization gate, Full State Verification.
//!
//! Source of truth: the persisted `materialization_plan_*.json` artifact on disk (the Loom
//! `MaterializationPlan` + the per-pair measured pair-gain), read back separately. Each case
//! constructs synthetic data with a *known* pair-gain structure and proves the gate materializes
//! exactly the interaction cross-terms that earn ≥ 0.05 measured bits.

use std::path::Path;

use calyx_assay::TrustTag;
use calyx_core::{FixedClock, SlotId};
use calyx_loom::{CrossTermKind, MaterializationAction, StaticPairGainGate, plan_cross_terms};
use calyx_poly::pair_gain_gate::{
    ERR_DEGENERATE_OUTCOME, PAIR_GAIN_BITS_THRESHOLD, PairGainMeasurement, compute_pair_gain_plan,
    read_pair_gain_plan, write_pair_gain_plan,
};
use calyx_poly::panel_diagnostics::PanelMatrix;
use serde_json::json;

// calyx-shared-module: path=fsv_support.rs alias=__calyx_shared_fsv_support_rs local=support visibility=private
use crate::__calyx_shared_fsv_support_rs as support;
// calyx-shared-module: path=synthetic_panels.rs alias=__calyx_shared_synthetic_panels_rs local=synthetic visibility=private
use crate::__calyx_shared_synthetic_panels_rs as synthetic;

use support::{named_fsv_root, reset_dir, write_blake3sums, write_json};
use synthetic::pair_gain_structure;

const PANEL_VERSION: u32 = 1;
const K: usize = 3;

fn find<'a>(ms: &'a [PairGainMeasurement], a: &str, b: &str) -> &'a PairGainMeasurement {
    ms.iter()
        .find(|m| (m.key_a == a && m.key_b == b) || (m.key_a == b && m.key_b == a))
        .unwrap_or_else(|| panic!("no measurement for pair ({a}, {b})"))
}

#[test]
fn issue208_pair_gain_gate_fsv() {
    let (root, _keep) = named_fsv_root("POLY_ISSUE208_FSV_ROOT", "poly-issue208-pair-gain-gate");
    reset_dir(&root);
    let clock = FixedClock::new(1_785_400_000);

    happy_gate_materializes_by_measured_gain(&root, &clock);
    edge_threshold_boundary_is_inclusive();
    edge_below_floor_is_lazy_provisional(&root, &clock);
    edge_degenerate_outcome_fails_closed(&root, &clock);

    write_blake3sums(&root);
}

/// Happy path: a ≥0.05-bit XOR pair is eager; a ~0-bit noise pair and a jointly-redundant strong pair
/// are lazy. Readback matches the constructed truth.
fn happy_gate_materializes_by_measured_gain(root: &Path, clock: &FixedClock) {
    let panel = pair_gain_structure(20_208, 300, true);
    let matrix = PanelMatrix::new(panel.keys, panel.columns, panel.anchors).expect("matrix");
    let record = compute_pair_gain_plan("crypto", PANEL_VERSION, &matrix, clock, K).expect("plan");

    let xor = find(&record.measurements, "xa", "xb");
    let noise = find(&record.measurements, "noise1", "noise2");
    let redundant = find(&record.measurements, "strong1", "strong2");
    eprintln!(
        "[#208 happy] xor: gain={:.3} left={:.3} right={:.3} pair={:.3} eager={} | \
         noise: gain={:.3} eager={} | redundant(strong): gain={:.3} left={:.3} right={:.3} pair={:.3} eager={} | trust={:?}",
        xor.pair_gain,
        xor.left_bits,
        xor.right_bits,
        xor.pair_bits,
        xor.interaction_eager,
        noise.pair_gain,
        noise.interaction_eager,
        redundant.pair_gain,
        redundant.left_bits,
        redundant.right_bits,
        redundant.pair_bits,
        redundant.interaction_eager,
        record.trust,
    );

    // XOR pair: individually uninformative, jointly determines the outcome → gain ≥ 0.05 → eager.
    assert!(
        xor.pair_gain >= PAIR_GAIN_BITS_THRESHOLD,
        "XOR pair-gain {} must clear the 0.05-bit floor",
        xor.pair_gain
    );
    assert!(
        xor.interaction_eager,
        "XOR interaction must be materialized eager"
    );
    assert!(!xor.provisional, "above-floor pair is not provisional");

    // Noise pair: no joint signal → below floor → lazy.
    assert!(!noise.interaction_eager, "noise interaction must be lazy");
    assert!(noise.pair_gain < PAIR_GAIN_BITS_THRESHOLD);

    // Jointly-redundant strong pair: each member predicts the outcome, but together they add nothing
    // beyond the stronger member → non-positive pair-gain → lazy.
    assert!(
        redundant.left_bits > 0.05 && redundant.right_bits > 0.05,
        "the redundant pair's members must each individually predict the outcome"
    );
    assert!(
        redundant.pair_gain < PAIR_GAIN_BITS_THRESHOLD,
        "jointly-redundant pair-gain {} must not clear the floor",
        redundant.pair_gain
    );
    assert!(
        !redundant.interaction_eager,
        "redundant interaction must be lazy"
    );

    // The Loom plan agrees with the measurements: every interaction eager-store corresponds to a
    // measured pair-gain ≥ 0.05, and agreement cross-terms are always eager.
    assert_eq!(
        record.trust,
        TrustTag::Trusted,
        "resolved anchors → Trusted"
    );
    let agreements_all_eager = record
        .plan
        .entries
        .iter()
        .filter(|e| e.kind == CrossTermKind::Agreement)
        .all(|e| e.action == MaterializationAction::EagerStore);
    assert!(
        agreements_all_eager,
        "agreement cross-terms are always eager"
    );
    for entry in record
        .plan
        .entries
        .iter()
        .filter(|e| e.kind == CrossTermKind::Interaction)
    {
        let m = find(
            &record.measurements,
            &keys(&record, entry.a),
            &keys(&record, entry.b),
        );
        let eager = entry.action == MaterializationAction::EagerStore;
        assert_eq!(
            eager,
            m.pair_gain >= PAIR_GAIN_BITS_THRESHOLD,
            "interaction eager/lazy must match the measured pair-gain for ({}, {})",
            m.key_a,
            m.key_b
        );
    }

    // Persist, read back, prove round-trip equality and physical existence.
    let path = write_pair_gain_plan(root, &record).expect("write plan");
    let readback = read_pair_gain_plan(&path).expect("read plan");
    assert_eq!(
        readback, record,
        "on-disk plan must equal the computed record"
    );
    assert!(path.exists());

    write_json(
        &root.join("happy_summary.json"),
        &json!({
            "artifact_path": path.display().to_string(),
            "interaction_eager_count": record.interaction_eager_count,
            "xor_pair_gain": xor.pair_gain,
            "noise_pair_gain": noise.pair_gain,
            "redundant_pair_gain": redundant.pair_gain,
            "trust": format!("{:?}", record.trust),
            "provenance_hash": record.provenance_hash,
        }),
    );
}

fn keys(
    record: &calyx_poly::pair_gain_gate::PairGainMaterializationRecord,
    slot: SlotId,
) -> String {
    record.slot_keys[slot.get() as usize].clone()
}

/// Edge: the gate boundary is inclusive at exactly 0.05 bits (the engine's contract Poly relies on).
fn edge_threshold_boundary_is_inclusive() {
    let slots = [SlotId::new(0), SlotId::new(1)];
    let at = plan_cross_terms(&slots, &StaticPairGainGate { gain_bits: 0.05 });
    let below = plan_cross_terms(&slots, &StaticPairGainGate { gain_bits: 0.0499 });
    let interaction_action = |plan: &calyx_loom::MaterializationPlan| {
        plan.entries
            .iter()
            .find(|e| e.kind == CrossTermKind::Interaction)
            .map(|e| e.action)
            .expect("interaction entry")
    };
    assert_eq!(
        interaction_action(&at),
        MaterializationAction::EagerStore,
        "exactly 0.05 bits must materialize eager (inclusive boundary)"
    );
    assert_eq!(
        interaction_action(&below),
        MaterializationAction::LazyCache,
        "0.0499 bits must stay lazy"
    );
}

/// Edge: below the MI sample floor every pair is lazy and Provisional — never eager on unproven signal.
fn edge_below_floor_is_lazy_provisional(root: &Path, clock: &FixedClock) {
    let panel = pair_gain_structure(30_208, 40, true); // < MIN_ASSAY_SAMPLES (50)
    let matrix = PanelMatrix::new(panel.keys, panel.columns, panel.anchors).expect("matrix");
    let record =
        compute_pair_gain_plan("belowfloor", PANEL_VERSION, &matrix, clock, K).expect("plan");
    eprintln!(
        "[#208 below-floor] all_provisional={} interaction_eager_count={} trust={:?}",
        record.measurements.iter().all(|m| m.provisional),
        record.interaction_eager_count,
        record.trust
    );
    assert!(
        record.measurements.iter().all(|m| m.provisional),
        "below floor: every pair must be provisional"
    );
    assert_eq!(
        record.interaction_eager_count, 0,
        "below floor: no interaction cross-term may be eager"
    );
    assert_eq!(record.trust, TrustTag::Provisional);
    assert!(
        record
            .plan
            .entries
            .iter()
            .filter(|e| e.kind == CrossTermKind::Interaction)
            .all(|e| e.action == MaterializationAction::LazyCache),
        "below floor: all interaction cross-terms lazy"
    );
    let path = write_pair_gain_plan(root, &record).expect("write");
    assert_eq!(read_pair_gain_plan(&path).expect("read"), record);
}

/// Edge: a single-class outcome fails closed — MI about a constant outcome is undefined.
fn edge_degenerate_outcome_fails_closed(root: &Path, clock: &FixedClock) {
    // All observations resolve the same way → outcome has no contrast.
    let mut panel = pair_gain_structure(40_208, 80, true);
    for (i, anchor) in panel.anchors.iter_mut().enumerate() {
        *anchor = synthetic::resolved_anchor(true, i);
    }
    let matrix = PanelMatrix::new(panel.keys, panel.columns, panel.anchors).expect("matrix");
    let err = compute_pair_gain_plan("degenerate", PANEL_VERSION, &matrix, clock, K)
        .expect_err("single-class outcome must fail closed");
    assert_eq!(err.code(), ERR_DEGENERATE_OUTCOME);
    write_json(
        &root.join("edge_degenerate_outcome.json"),
        &json!({ "code": err.code(), "message": err.message() }),
    );
}
