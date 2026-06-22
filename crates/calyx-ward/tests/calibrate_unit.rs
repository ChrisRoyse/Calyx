use std::collections::BTreeMap;

use calyx_core::{FixedClock, SlotId};
use calyx_ward::{
    CalibrationInput, ESTIMATOR, GuardId, GuardPolicy, GuardProfile, MIN_BAD_SCORES, NoveltyAction,
    SlotKind, WardError, calibrate, calibrate_slot,
};
use proptest::prelude::*;
use serde_json::json;
use sha2::{Digest, Sha256};

const GUARD_UUID: &str = "018f48a4-9a79-74d2-8a5c-9ad7f6b8c101";

#[test]
fn calibrates_identity_slot_with_bounded_far() {
    let clock = FixedClock::new(1_785_400_000);
    let input = calibration_input(slot(1), SlotKind::Identity, 0.01);

    let (tau, meta) = calibrate_slot(&input, 0.05, &clock).expect("calibrate");

    assert!((0.55..=0.75).contains(&tau));
    assert!(meta.far <= 0.01);
    assert_eq!(meta.estimator, ESTIMATOR);
    assert_eq!(meta.confidence, 0.95);
    assert_eq!(meta.ts, 1_785_400_000);
}

#[test]
fn identity_tau_is_at_least_stylistic_tau() {
    let clock = FixedClock::new(1_785_400_000);

    let (identity_tau, _) = calibrate_slot(
        &calibration_input(slot(1), SlotKind::Identity, 0.01),
        0.05,
        &clock,
    )
    .expect("identity");
    let (style_tau, _) = calibrate_slot(
        &calibration_input(slot(2), SlotKind::Stylistic, 0.05),
        0.05,
        &clock,
    )
    .expect("style");

    assert!(identity_tau > style_tau);
}

proptest! {
    #[test]
    fn achieved_far_never_exceeds_target(
        mut bad_scores in proptest::collection::vec(0.0f32..1.0, MIN_BAD_SCORES..100),
        target_far in 0.0f32..0.03,
    ) {
        bad_scores.sort_by(|left, right| left.total_cmp(right));
        let input = CalibrationInput {
            slot: slot(1),
            good_scores: vec![0.9; MIN_BAD_SCORES],
            bad_scores,
            slot_kind: SlotKind::Content,
            target_far,
        };

        let (_, meta) = calibrate_slot(&input, 0.05, &FixedClock::new(1))
            .expect("calibrate");

        prop_assert!(meta.far <= target_far + f32::EPSILON);
    }
}

#[test]
fn exactly_min_bad_scores_is_allowed() {
    let mut input = calibration_input(slot(1), SlotKind::Content, 0.03);
    input.bad_scores.truncate(MIN_BAD_SCORES);

    let result = calibrate_slot(&input, 0.05, &FixedClock::new(1));

    assert!(result.is_ok());
}

#[test]
fn below_min_bad_scores_fails_provisional() {
    let mut input = calibration_input(slot(1), SlotKind::Content, 0.03);
    input.bad_scores.truncate(MIN_BAD_SCORES - 1);

    let error = calibrate_slot(&input, 0.05, &FixedClock::new(1)).expect_err("quorum");

    assert_eq!(
        error,
        WardError::InsufficientCalibrationData {
            n: MIN_BAD_SCORES - 1,
            min: MIN_BAD_SCORES,
        }
    );
    assert_eq!(error.code(), "CALYX_GUARD_PROVISIONAL");
}

#[test]
fn all_high_bad_scores_calibrate_to_high_tau_with_zero_far() {
    let input = CalibrationInput {
        slot: slot(1),
        good_scores: vec![1.0; 100],
        bad_scores: vec![0.99; 100],
        slot_kind: SlotKind::Identity,
        target_far: 0.01,
    };

    let (tau, meta) = calibrate_slot(&input, 0.05, &FixedClock::new(1)).expect("calibrate");

    assert!(tau > 0.99);
    assert_eq!(meta.far, 0.0);
}

#[test]
fn target_far_zero_uses_max_bad_score() {
    let input = calibration_input(slot(1), SlotKind::Identity, 0.0);

    let (tau, meta) = calibrate_slot(&input, 0.05, &FixedClock::new(1)).expect("calibrate");
    let max_bad = *input
        .bad_scores
        .iter()
        .max_by(|a, b| a.total_cmp(b))
        .unwrap();

    assert!(tau > max_bad);
    assert_eq!(meta.far, 0.0);
}

#[test]
fn ties_at_quantile_do_not_underreport_false_acceptance() {
    let input = CalibrationInput {
        slot: slot(1),
        good_scores: vec![0.9; 100],
        bad_scores: (0..98).map(|_| 0.10).chain([0.90, 0.90]).collect(),
        slot_kind: SlotKind::Identity,
        target_far: 0.01,
    };

    let (tau, meta) = calibrate_slot(&input, 0.05, &FixedClock::new(1)).expect("calibrate");

    assert!(tau > 0.90);
    assert_eq!(meta.far, 0.0);
}

#[test]
fn identity_slot_rejects_looser_than_default_far() {
    let input = calibration_input(slot(1), SlotKind::Identity, 0.05);

    let error = calibrate_slot(&input, 0.05, &FixedClock::new(1)).expect_err("loose far");

    assert_eq!(error.code(), "CALYX_GUARD_PROVISIONAL");
    assert!(error.to_string().contains("target_far exceeds slot_kind"));
}

#[test]
fn calibrate_updates_profile_with_merged_provenance() {
    let clock = FixedClock::new(1_785_400_000);
    let profile = profile_template();

    let calibrated = calibrate(
        profile,
        vec![
            calibration_input(slot(1), SlotKind::Identity, 0.01),
            calibration_input(slot(2), SlotKind::Stylistic, 0.05),
        ],
        0.05,
        &clock,
    )
    .expect("calibrate profile");

    assert!(calibrated.is_calibrated());
    assert!(calibrated.tau_for(&slot(1)).unwrap() > calibrated.tau_for(&slot(2)).unwrap());
    assert_eq!(calibrated.required_slots, vec![slot(1), slot(2)]);
    assert_eq!(
        calibrated.calibration.as_ref().unwrap().estimator,
        ESTIMATOR
    );
    assert_eq!(calibrated.calibration.as_ref().unwrap().ts, 1_785_400_000);
}

#[test]
fn calibrate_preserves_distinct_per_slot_bounds() {
    let clock = FixedClock::new(1_785_400_000);
    let mut identity_input = confidence_supported_input(slot(1), SlotKind::Identity, 0.01);
    let mut stylistic_input = confidence_supported_input(slot(2), SlotKind::Stylistic, 0.05);
    identity_input.good_scores = vec![0.59; 100];
    stylistic_input.good_scores = vec![0.59; 100];
    let calibrated = calibrate(
        profile_template(),
        vec![identity_input, stylistic_input],
        0.05,
        &clock,
    )
    .expect("calibrate profile");

    let meta = calibrated.calibration.as_ref().expect("profile meta");
    let identity = meta.per_slot.get(&slot(1)).expect("identity slot meta");
    let stylistic = meta.per_slot.get(&slot(2)).expect("style slot meta");

    assert!(identity.far < stylistic.far);
    assert!(identity.frr > stylistic.frr);
    assert_eq!(meta.far, stylistic.far);
    assert_eq!(meta.frr, identity.frr);
    assert_eq!(identity.estimator, ESTIMATOR);
    assert_eq!(stylistic.ts, 1_785_400_000);
}

#[test]
fn alpha_changes_tau_when_sample_supports_confidence_bound() {
    let input = confidence_supported_input(slot(8), SlotKind::Stylistic, 0.05);
    let clock = FixedClock::new(1_785_400_000);

    let (strict_tau, strict_meta) = calibrate_slot(&input, 0.01, &clock).expect("strict alpha");
    let (loose_tau, loose_meta) = calibrate_slot(&input, 0.20, &clock).expect("loose alpha");

    assert!(strict_tau > loose_tau);
    assert!(strict_meta.far < loose_meta.far);
    assert_eq!(strict_meta.confidence, 0.99);
    assert_eq!(loose_meta.confidence, 0.80);
    assert_ne!(strict_meta.corpus_hash, loose_meta.corpus_hash);
}

#[test]
#[ignore = "manual FSV fixture; set CALYX_WARD_CALIBRATE_FSV_DIR"]
fn calibrate_fsv_fixture_writes_readback_artifacts() {
    let root = std::env::var("CALYX_WARD_CALIBRATE_FSV_DIR")
        .expect("CALYX_WARD_CALIBRATE_FSV_DIR is required");
    std::fs::create_dir_all(&root).expect("create fsv root");
    let clock = FixedClock::new(1_785_400_000);
    let identity = calibration_input(slot(1), SlotKind::Identity, 0.01);
    let stylistic = calibration_input(slot(2), SlotKind::Stylistic, 0.05);
    let (identity_tau, identity_meta) = calibrate_slot(&identity, 0.05, &clock).expect("identity");
    let (style_tau, style_meta) = calibrate_slot(&stylistic, 0.05, &clock).expect("style");
    let calibrated =
        calibrate(profile_template(), vec![identity, stylistic], 0.05, &clock).expect("profile");
    let mut insufficient = calibration_input(slot(3), SlotKind::Content, 0.03);
    insufficient.bad_scores.truncate(MIN_BAD_SCORES - 1);
    let insufficient_error = calibrate_slot(&insufficient, 0.05, &clock).expect_err("insufficient");
    let all_high = CalibrationInput {
        slot: slot(4),
        good_scores: vec![1.0; 100],
        bad_scores: vec![0.99; 100],
        slot_kind: SlotKind::Identity,
        target_far: 0.01,
    };
    let (all_high_tau, all_high_meta) = calibrate_slot(&all_high, 0.05, &clock).expect("all high");
    let tie_boundary = CalibrationInput {
        slot: slot(5),
        good_scores: vec![0.9; 100],
        bad_scores: (0..98).map(|_| 0.10).chain([0.90, 0.90]).collect(),
        slot_kind: SlotKind::Identity,
        target_far: 0.01,
    };
    let (tie_tau, tie_meta) = calibrate_slot(&tie_boundary, 0.05, &clock).expect("tie boundary");
    let zero_far = calibration_input(slot(6), SlotKind::Identity, 0.0);
    let zero_far_max_bad = zero_far
        .bad_scores
        .iter()
        .copied()
        .max_by(|a, b| a.total_cmp(b))
        .expect("max bad score");
    let (zero_far_tau, zero_far_meta) = calibrate_slot(&zero_far, 0.05, &clock).expect("zero far");
    let loose_identity = calibration_input(slot(7), SlotKind::Identity, 0.05);
    let loose_identity_error =
        calibrate_slot(&loose_identity, 0.05, &clock).expect_err("loose identity");
    let alpha_input = confidence_supported_input(slot(8), SlotKind::Stylistic, 0.05);
    let (strict_alpha_tau, strict_alpha_meta) =
        calibrate_slot(&alpha_input, 0.01, &clock).expect("strict alpha");
    let (loose_alpha_tau, loose_alpha_meta) =
        calibrate_slot(&alpha_input, 0.20, &clock).expect("loose alpha");

    write_json(
        &root,
        "calibration.json",
        &json!({
            "profile": calibrated,
            "estimator": ESTIMATOR,
        }),
    );
    write_json(
        &root,
        "identity-style-comparison.json",
        &json!({
            "identity_tau": identity_tau,
            "identity_far": identity_meta.far,
            "style_tau": style_tau,
            "style_far": style_meta.far,
            "profile_slot1_far": calibrated
                .calibration
                .as_ref()
                .and_then(|meta| meta.per_slot.get(&slot(1)))
                .map(|meta| meta.far),
            "profile_slot2_far": calibrated
                .calibration
                .as_ref()
                .and_then(|meta| meta.per_slot.get(&slot(2)))
                .map(|meta| meta.far),
            "identity_tau_gt_style_tau": identity_tau > style_tau,
        }),
    );
    write_json(
        &root,
        "insufficient-error.json",
        &error_json(&insufficient_error),
    );
    write_json(
        &root,
        "all-high-bad-scores.json",
        &json!({
            "tau": all_high_tau,
            "far": all_high_meta.far,
        }),
    );
    write_json(
        &root,
        "quantile-ties.json",
        &json!({
            "tie_score": 0.90,
            "tau": tie_tau,
            "tau_above_tie_score": tie_tau > 0.90,
            "far": tie_meta.far,
        }),
    );
    write_json(
        &root,
        "zero-target-far.json",
        &json!({
            "max_bad": zero_far_max_bad,
            "tau": zero_far_tau,
            "tau_above_max_bad": zero_far_tau > zero_far_max_bad,
            "far": zero_far_meta.far,
        }),
    );
    write_json(
        &root,
        "alpha-confidence-bound.json",
        &json!({
            "target_far": alpha_input.target_far,
            "bad_count": alpha_input.bad_scores.len(),
            "strict_alpha": 0.01,
            "strict_confidence": strict_alpha_meta.confidence,
            "strict_tau": strict_alpha_tau,
            "strict_far": strict_alpha_meta.far,
            "strict_corpus_hash": hash_hex(&strict_alpha_meta.corpus_hash),
            "loose_alpha": 0.20,
            "loose_confidence": loose_alpha_meta.confidence,
            "loose_tau": loose_alpha_tau,
            "loose_far": loose_alpha_meta.far,
            "loose_corpus_hash": hash_hex(&loose_alpha_meta.corpus_hash),
            "strict_tau_gt_loose_tau": strict_alpha_tau > loose_alpha_tau,
            "strict_far_lt_loose_far": strict_alpha_meta.far < loose_alpha_meta.far,
            "corpus_hashes_differ": strict_alpha_meta.corpus_hash != loose_alpha_meta.corpus_hash,
        }),
    );
    write_json(
        &root,
        "loose-identity-error.json",
        &error_json(&loose_identity_error),
    );
    write_sha_manifest(&root);

    println!(
        "FSV_CALIBRATE estimator={} identity_tau={:.6} style_tau={:.6} identity_far={:.6} tie_far={:.6} zero_far={:.6} strict_alpha_tau={:.6} loose_alpha_tau={:.6} insufficient_code={} loose_identity_code={}",
        ESTIMATOR,
        identity_tau,
        style_tau,
        identity_meta.far,
        tie_meta.far,
        zero_far_meta.far,
        strict_alpha_tau,
        loose_alpha_tau,
        insufficient_error.code(),
        loose_identity_error.code()
    );
}

fn calibration_input(slot: SlotId, slot_kind: SlotKind, target_far: f32) -> CalibrationInput {
    CalibrationInput {
        slot,
        good_scores: (0..100).map(|i| 0.80 + i as f32 * 0.001).collect(),
        bad_scores: (0..100).map(|i| 0.30 + i as f32 * 0.003).collect(),
        slot_kind,
        target_far,
    }
}

fn confidence_supported_input(
    slot: SlotId,
    slot_kind: SlotKind,
    target_far: f32,
) -> CalibrationInput {
    CalibrationInput {
        slot,
        good_scores: (0..1_000).map(|i| 0.80 + i as f32 * 0.0001).collect(),
        bad_scores: (0..1_000).map(|i| 0.30 + i as f32 * 0.0003).collect(),
        slot_kind,
        target_far,
    }
}

fn profile_template() -> GuardProfile {
    GuardProfile {
        guard_id: guard_id(),
        panel_version: 42,
        domain: "synthetic".to_string(),
        tau: BTreeMap::new(),
        required_slots: Vec::new(),
        policy: GuardPolicy::AllRequired,
        calibration: None,
        novelty_action: NoveltyAction::Quarantine,
    }
}

fn error_json(error: &WardError) -> serde_json::Value {
    json!({
        "code": error.code(),
        "message": error.to_string(),
    })
}

fn write_json<T: serde::Serialize>(root: &str, name: &str, value: &T) {
    let path = std::path::Path::new(root).join(name);
    let file = std::fs::File::create(path).expect("create fsv json");
    serde_json::to_writer_pretty(file, value).expect("write fsv json");
}

fn write_sha_manifest(root: &str) {
    let root = std::path::Path::new(root);
    let mut lines = Vec::new();
    for entry in std::fs::read_dir(root).expect("read fsv root") {
        let path = entry.expect("dir entry").path();
        if path.is_file() && path.file_name().unwrap() != "sha256-manifest.txt" {
            let bytes = std::fs::read(&path).expect("read fsv file");
            lines.push(format!(
                "{:x}  {}\n",
                Sha256::digest(bytes),
                path.file_name().unwrap().to_string_lossy()
            ));
        }
    }
    lines.sort();
    std::fs::write(root.join("sha256-manifest.txt"), lines.concat()).expect("write sha manifest");
}

fn hash_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn guard_id() -> GuardId {
    GUARD_UUID.parse().expect("guard id")
}

const fn slot(value: u16) -> SlotId {
    SlotId::new(value)
}
