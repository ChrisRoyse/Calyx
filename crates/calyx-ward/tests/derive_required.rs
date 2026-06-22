use std::collections::BTreeMap;

use calyx_assay::{CoverageMask, per_sensor_attribution_with_coverage};
use calyx_core::{
    AnchorKind, Asymmetry, ConfidenceInterval, CxId, LedgerRef, LensId, Modality, Panel,
    QuantPolicy, Signal, Slot, SlotId, SlotKey, SlotShape, SlotState,
};
use calyx_ward::{
    GuardId, GuardPolicy, GuardProfile, LOAD_BEARING_MIN_BITS, NoveltyAction,
    RequiredSlotDerivation, RequiredSlotObservation, WardError, derive_required_profile,
    derive_required_slots, derive_required_slots_for_observations,
};
use serde_json::json;

const GUARD_UUID: &str = "018f48a4-9a79-74d2-8a5c-9ad7f6b8c101";

#[test]
fn derives_required_slots_from_assay_bits_for_anchor() {
    let panel = panel_with_bits(&[
        (slot(1), Some(0.071), SlotState::Active),
        (slot(2), Some(0.049), SlotState::Active),
        (slot(3), None, SlotState::Active),
    ]);
    let config = RequiredSlotDerivation::assay_bits(AnchorKind::Reward);

    let evidence = derive_required_slots(&panel, &config).expect("derive");
    let profile = derive_required_profile(sample_profile(), &panel, &config).expect("profile");

    assert_eq!(evidence, vec![evidence_slot(1, 0.071)]);
    assert_eq!(profile.required_slots, vec![slot(1)]);
    assert_eq!(profile.tau_for(&slot(1)), Some(0.91));
    assert_eq!(profile.panel_version, u64::from(panel.version));
}

#[test]
fn exact_threshold_is_load_bearing() {
    let panel = panel_with_bits(&[
        (slot(1), Some(LOAD_BEARING_MIN_BITS), SlotState::Active),
        (
            slot(2),
            Some(LOAD_BEARING_MIN_BITS - 0.001),
            SlotState::Active,
        ),
    ]);

    let profile = derive_required_profile(
        sample_profile(),
        &panel,
        &RequiredSlotDerivation::assay_bits(AnchorKind::Reward),
    )
    .expect("profile");

    assert_eq!(profile.required_slots, vec![slot(1)]);
}

#[test]
fn manual_override_replaces_assay_derived_slots() {
    let panel = panel_with_bits(&[
        (slot(1), Some(0.080), SlotState::Active),
        (slot(2), Some(0.010), SlotState::Active),
    ]);
    let config = RequiredSlotDerivation::manual(AnchorKind::Reward, vec![slot(2), slot(2)]);

    let profile = derive_required_profile(sample_profile(), &panel, &config).expect("profile");

    assert_eq!(profile.required_slots, vec![slot(2)]);
    assert_eq!(profile.tau_for(&slot(2)), Some(0.70));
}

#[test]
fn manual_empty_required_slots_fail_closed() {
    let panel = panel_with_bits(&[(slot(1), Some(0.080), SlotState::Active)]);
    let config = RequiredSlotDerivation::manual(AnchorKind::Reward, Vec::new());

    let error = derive_required_profile(sample_profile(), &panel, &config)
        .expect_err("empty manual required slots");

    assert_eq!(
        error,
        WardError::InvalidRequiredSlotDerivation {
            reason: "manual required slots must be non-empty",
        }
    );
    assert_eq!(error.code(), "CALYX_GUARD_PROVISIONAL");
}

#[test]
fn no_derived_slots_fails_closed_without_manual_override() {
    let panel = panel_with_bits(&[(slot(1), Some(0.010), SlotState::Active)]);

    let error = derive_required_profile(
        sample_profile(),
        &panel,
        &RequiredSlotDerivation::assay_bits(AnchorKind::Reward),
    )
    .expect_err("no load-bearing slots");

    assert_eq!(
        error,
        WardError::InvalidRequiredSlotDerivation {
            reason: "no load-bearing slots for anchor",
        }
    );
    assert_eq!(error.code(), "CALYX_GUARD_PROVISIONAL");
}

#[test]
fn non_finite_bits_fail_closed() {
    let panel = panel_with_bits(&[(slot(1), Some(f32::NAN), SlotState::Active)]);

    let error = derive_required_slots(
        &panel,
        &RequiredSlotDerivation::assay_bits(AnchorKind::Reward),
    )
    .expect_err("invalid bits");

    assert_eq!(
        error,
        WardError::InvalidRequiredSlotDerivation {
            reason: "slot bits_about must be finite",
        }
    );
}

#[test]
fn inactive_high_bit_slot_is_not_required() {
    let panel = panel_with_bits(&[
        (slot(1), Some(0.090), SlotState::Parked),
        (slot(2), Some(0.060), SlotState::Active),
    ]);

    let profile = derive_required_profile(
        sample_profile(),
        &panel,
        &RequiredSlotDerivation::assay_bits(AnchorKind::Reward),
    )
    .expect("profile");

    assert_eq!(profile.required_slots, vec![slot(2)]);
}

#[test]
fn coverage_masks_remove_unobserved_slots_from_ward_required_set() {
    let covered = cx(0xA1);
    let uncovered = cx(0xB2);
    let attributions = per_sensor_attribution_with_coverage(
        &[
            (slot(1), 0.07, CoverageMask::Full),
            (slot(2), 0.40, CoverageMask::partial(2, [covered]).unwrap()),
        ],
        LOAD_BEARING_MIN_BITS,
    );
    let config = RequiredSlotDerivation::assay_bits(AnchorKind::Reward);

    let covered_observations = observations_for(&attributions, covered);
    let uncovered_observations = observations_for(&attributions, uncovered);
    let covered_required =
        derive_required_slots_for_observations(&covered_observations, &config).expect("covered");
    let uncovered_required =
        derive_required_slots_for_observations(&uncovered_observations, &config)
            .expect("uncovered");

    assert_eq!(
        covered_required,
        vec![evidence_slot(1, 0.07), evidence_slot(2, 0.40)]
    );
    assert_eq!(uncovered_required, vec![evidence_slot(1, 0.07)]);
}

#[test]
#[ignore = "manual FSV fixture; set CALYX_WARD_REQUIRED_FSV_DIR"]
fn required_slots_fsv_fixture_writes_readback_artifacts() {
    let root = std::env::var("CALYX_WARD_REQUIRED_FSV_DIR")
        .expect("CALYX_WARD_REQUIRED_FSV_DIR is required");
    std::fs::create_dir_all(&root).expect("create fsv root");

    let panel = panel_with_bits(&[
        (slot(1), Some(0.071), SlotState::Active),
        (slot(2), Some(0.049), SlotState::Active),
        (slot(3), None, SlotState::Active),
    ]);
    let config = RequiredSlotDerivation::assay_bits(AnchorKind::Reward);
    let evidence = derive_required_slots(&panel, &config).expect("derive");
    let profile = derive_required_profile(sample_profile(), &panel, &config).expect("profile");
    let boundary = derive_required_profile(
        sample_profile(),
        &panel_with_bits(&[
            (slot(4), Some(0.050), SlotState::Active),
            (slot(5), Some(0.049), SlotState::Active),
        ]),
        &config,
    )
    .expect("boundary");
    let manual = derive_required_profile(
        sample_profile(),
        &panel,
        &RequiredSlotDerivation::manual(AnchorKind::Reward, vec![slot(2), slot(2)]),
    )
    .expect("manual");
    let no_required = derive_required_profile(
        sample_profile(),
        &panel_with_bits(&[(slot(8), Some(0.010), SlotState::Active)]),
        &config,
    )
    .expect_err("no required");
    let invalid = derive_required_slots(
        &panel_with_bits(&[(slot(9), Some(f32::NAN), SlotState::Active)]),
        &config,
    )
    .expect_err("invalid bits");

    write_json(&root, "derived-evidence.json", &evidence);
    write_json(&root, "derived-profile.json", &profile);
    write_json(&root, "boundary-profile.json", &boundary);
    write_json(&root, "manual-profile.json", &manual);
    write_json(
        &root,
        "edge-errors.json",
        &json!([
            {"code": no_required.code(), "message": no_required.to_string()},
            {"code": invalid.code(), "message": invalid.to_string()}
        ]),
    );

    println!(
        "FSV_REQUIRED derived={:?} boundary={:?} manual={:?}",
        profile.required_slots, boundary.required_slots, manual.required_slots
    );
}

fn panel_with_bits(entries: &[(SlotId, Option<f32>, SlotState)]) -> Panel {
    Panel {
        version: 77,
        slots: entries
            .iter()
            .map(|(slot_id, bits, state)| slot_with_bits(*slot_id, *bits, *state))
            .collect(),
        created_at: 1_785_500_000,
        kernel_ref: Some(LedgerRef {
            seq: 1,
            hash: [1; 32],
        }),
        guard_ref: None,
    }
}

fn slot_with_bits(slot_id: SlotId, bits: Option<f32>, state: SlotState) -> Slot {
    let mut bits_about = BTreeMap::new();
    if let Some(bits) = bits {
        bits_about.insert(
            AnchorKind::Reward,
            Signal {
                bits,
                ci: ConfidenceInterval {
                    low: bits - 0.001,
                    high: bits + 0.001,
                },
                n: 80,
                estimator: "synthetic_assay".to_string(),
                ts: 1_785_500_001,
            },
        );
    }
    Slot {
        slot_id,
        slot_key: SlotKey::new(slot_id, format!("slot-{slot_id}")),
        lens_id: LensId::from_bytes([slot_id.get() as u8; 16]),
        shape: SlotShape::Dense(2),
        modality: Modality::Text,
        asymmetry: Asymmetry::None,
        quant: QuantPolicy::None,
        resource: Default::default(),
        axis: Some("synthetic".to_string()),
        retrieval_only: false,
        excluded_from_dedup: false,
        bits_about,
        state,
        added_at_panel_version: 77,
    }
}

fn sample_profile() -> GuardProfile {
    let mut tau = BTreeMap::new();
    tau.insert(slot(1), 0.91);
    GuardProfile {
        guard_id: GUARD_UUID.parse::<GuardId>().expect("guard id"),
        panel_version: 1,
        domain: "synthetic".to_string(),
        tau,
        required_slots: Vec::new(),
        policy: GuardPolicy::AllRequired,
        calibration: None,
        novelty_action: NoveltyAction::Quarantine,
    }
}

fn evidence_slot(slot_id: u16, bits: f32) -> calyx_ward::RequiredSlotEvidence {
    calyx_ward::RequiredSlotEvidence {
        slot: slot(slot_id),
        bits,
    }
}

fn observations_for(
    attributions: &[calyx_assay::SlotAttribution],
    cx: CxId,
) -> Vec<RequiredSlotObservation> {
    attributions
        .iter()
        .map(|attribution| RequiredSlotObservation {
            slot: attribution.slot,
            bits: attribution.marginal_bits,
            observed: attribution.is_observed_for(cx),
        })
        .collect()
}

fn write_json<T: serde::Serialize>(root: &str, name: &str, value: &T) {
    let path = std::path::Path::new(root).join(name);
    let file = std::fs::File::create(path).expect("create fsv json");
    serde_json::to_writer_pretty(file, value).expect("write fsv json");
}

const fn slot(value: u16) -> SlotId {
    SlotId::new(value)
}

fn cx(byte: u8) -> CxId {
    CxId::from_bytes([byte; 16])
}
