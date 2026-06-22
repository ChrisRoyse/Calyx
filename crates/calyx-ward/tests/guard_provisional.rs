use std::collections::BTreeMap;

use calyx_core::SlotId;
use calyx_ward::{
    CalibrationMeta, DEFAULT_TAU, GuardId, GuardPolicy, GuardProfile, MatchedSlots, NoveltyAction,
    ProducedSlots, SlotCalibrationMeta, WardError, guard, guard_non_high_stakes,
    guard_result_with_stakes,
};
use serde_json::json;
use sha2::{Digest, Sha256};

const GUARD_UUID: &str = "018f48a4-9a79-74d2-8a5c-9ad7f6b8c101";

#[test]
fn uncalibrated_high_stakes_refuses_provisional_before_slot_math() {
    let (profile, produced, matched) = uncalibrated_missing_tau_case();

    let error = guard(&profile, &produced, &matched, true).expect_err("provisional refusal");
    let formatted = error.to_string();

    assert_eq!(
        error,
        WardError::Provisional {
            guard_id: guard_id()
        }
    );
    assert_eq!(error.code(), "CALYX_GUARD_PROVISIONAL");
    assert!(formatted.contains("CALYX_GUARD_PROVISIONAL"));
    assert!(formatted.contains(GUARD_UUID));
    assert!(formatted.contains("calibrate before high-stakes use"));
    assert!(formatted.contains("anchored set >=50 examples"));
}

#[test]
fn uncalibrated_non_high_stakes_proceeds_with_provisional_verdict() {
    let (profile, produced, matched) = uncalibrated_missing_tau_case();

    let verdict = guard(&profile, &produced, &matched, false).expect("non-high-stakes verdict");

    assert!(verdict.provisional);
    assert!(verdict.overall_pass);
    assert_eq!(verdict.per_slot.len(), 1);
    assert_close(verdict.per_slot[0].cos, 0.75);
    assert_close(verdict.per_slot[0].tau, DEFAULT_TAU);
}

#[test]
fn calibrated_high_stakes_proceeds_without_provisional_flag() {
    let (profile, produced, matched) = calibrated_case(0.70);

    let verdict = guard(&profile, &produced, &matched, true).expect("calibrated verdict");

    assert!(!verdict.provisional);
    assert!(verdict.overall_pass);
    assert_close(verdict.per_slot[0].cos, 0.75);
    assert_close(verdict.per_slot[0].tau, 0.70);
}

#[test]
fn calibrated_non_high_stakes_proceeds_without_provisional_flag() {
    let (profile, produced, matched) = calibrated_case(0.70);

    let verdict = guard_non_high_stakes(&profile, &produced, &matched).expect("alias verdict");

    assert!(!verdict.provisional);
    assert!(verdict.overall_pass);
}

#[test]
fn calibrated_missing_tau_high_stakes_refuses_slot_provenance() {
    let (mut profile, produced, matched) = calibrated_case(0.70);
    profile.tau.clear();

    let error = guard(&profile, &produced, &matched, true).expect_err("missing tau");

    assert_eq!(
        error,
        WardError::MissingSlotCalibration {
            guard_id: guard_id(),
            slot: slot(1),
        }
    );
    assert_eq!(error.code(), "CALYX_GUARD_PROVISIONAL");
}

#[test]
fn calibrated_non_high_stakes_empty_tau_still_uses_cold_start() {
    let (mut profile, produced, matched) = calibrated_case(0.70);
    profile.tau.clear();

    let verdict = guard(&profile, &produced, &matched, false).expect("empty tau verdict");

    assert!(!verdict.provisional);
    assert!(verdict.overall_pass);
    assert_close(verdict.per_slot[0].tau, DEFAULT_TAU);
}

#[test]
fn profile_level_only_calibration_high_stakes_refuses_required_slot() {
    let mut profile = base_profile(Some(profile_level_calibration()));
    profile.tau.insert(slot(1), 0.70);
    let produced = produced_slots();
    let matched = matched_slots(0.75);

    let error = guard(&profile, &produced, &matched, true).expect_err("missing per-slot meta");

    assert_eq!(
        error,
        WardError::MissingSlotCalibration {
            guard_id: guard_id(),
            slot: slot(1),
        }
    );
    assert!(error.to_string().contains("required slot 1"));
}

#[test]
fn guard_result_with_stakes_propagates_provisional_before_ood_wrapping() {
    let (profile, produced, matched) = uncalibrated_missing_tau_case();

    let error = guard_result_with_stakes(&profile, &produced, &matched, true)
        .expect_err("provisional refusal");

    assert_eq!(
        error,
        WardError::Provisional {
            guard_id: guard_id()
        }
    );
}

#[test]
#[ignore = "manual FSV fixture; set CALYX_WARD_PROVISIONAL_FSV_DIR"]
fn guard_provisional_fsv_fixture_writes_readback_artifacts() {
    let root = std::env::var("CALYX_WARD_PROVISIONAL_FSV_DIR")
        .expect("CALYX_WARD_PROVISIONAL_FSV_DIR is required");
    std::fs::create_dir_all(&root).expect("create fsv root");

    let (uncalibrated, produced, matched) = uncalibrated_missing_tau_case();
    let high_stakes = guard(&uncalibrated, &produced, &matched, true)
        .expect_err("high-stakes provisional refusal");
    let non_high_stakes = guard(&uncalibrated, &produced, &matched, false)
        .expect("non-high-stakes provisional verdict");

    let (calibrated, calibrated_produced, calibrated_matched) = calibrated_case(0.70);
    let calibrated_high_stakes =
        guard(&calibrated, &calibrated_produced, &calibrated_matched, true)
            .expect("calibrated high-stakes verdict");

    let mut missing_tau_profile = calibrated.clone();
    missing_tau_profile.tau.clear();
    let missing_tau_error = guard(
        &missing_tau_profile,
        &calibrated_produced,
        &calibrated_matched,
        true,
    )
    .expect_err("calibrated missing tau");
    let mut profile_level_only = base_profile(Some(profile_level_calibration()));
    profile_level_only.tau.insert(slot(1), 0.70);
    let profile_level_only_error = guard(
        &profile_level_only,
        &calibrated_produced,
        &calibrated_matched,
        true,
    )
    .expect_err("profile-level-only calibration");

    write_json(&root, "high-stakes-error.json", &error_json(&high_stakes));
    write_json(&root, "non-high-stakes-provisional.json", &non_high_stakes);
    write_json(
        &root,
        "calibrated-high-stakes.json",
        &calibrated_high_stakes,
    );
    write_json(
        &root,
        "calibrated-missing-tau-error.json",
        &error_json(&missing_tau_error),
    );
    write_json(
        &root,
        "profile-level-only-error.json",
        &error_json(&profile_level_only_error),
    );
    write_json(
        &root,
        "case-summary.json",
        &json!({
            "guard_id": GUARD_UUID,
            "high_stakes_code": high_stakes.code(),
            "high_stakes_message": high_stakes.to_string(),
            "non_high_stakes_provisional": non_high_stakes.provisional,
            "non_high_stakes_tau": non_high_stakes.per_slot[0].tau,
            "calibrated_high_stakes_provisional": calibrated_high_stakes.provisional,
            "calibrated_high_stakes_tau": calibrated_high_stakes.per_slot[0].tau,
            "calibrated_slot_meta_count": calibrated
                .calibration
                .as_ref()
                .map(|meta| meta.per_slot.len()),
            "missing_tau_code": missing_tau_error.code(),
            "profile_level_only_code": profile_level_only_error.code(),
        }),
    );
    write_sha_manifest(&root);

    println!(
        "FSV_PH38_T02 high_stakes_code={} non_high_stakes_provisional={} non_high_stakes_tau={:.3} calibrated_provisional={} calibrated_tau={:.3} missing_tau_code={} profile_only_code={}",
        high_stakes.code(),
        non_high_stakes.provisional,
        non_high_stakes.per_slot[0].tau,
        calibrated_high_stakes.provisional,
        calibrated_high_stakes.per_slot[0].tau,
        missing_tau_error.code(),
        profile_level_only_error.code(),
    );
    println!("FSV_PH38_T02_MESSAGE {}", high_stakes);
}

fn uncalibrated_missing_tau_case() -> (GuardProfile, ProducedSlots, MatchedSlots) {
    let mut profile = base_profile(None);
    profile.tau.clear();
    (profile, produced_slots(), matched_slots(0.75))
}

fn calibrated_case(tau: f32) -> (GuardProfile, ProducedSlots, MatchedSlots) {
    let mut profile = base_profile(Some(calibration()));
    profile.tau.insert(slot(1), tau);
    (profile, produced_slots(), matched_slots(0.75))
}

fn base_profile(calibration: Option<CalibrationMeta>) -> GuardProfile {
    GuardProfile {
        guard_id: guard_id(),
        panel_version: 42,
        domain: "synthetic".to_string(),
        tau: [(slot(1), 0.70)].into_iter().collect(),
        required_slots: vec![slot(1)],
        policy: GuardPolicy::AllRequired,
        calibration,
        novelty_action: NoveltyAction::Quarantine,
    }
}

fn calibration() -> CalibrationMeta {
    let mut per_slot = BTreeMap::new();
    per_slot.insert(slot(1), slot_calibration());
    CalibrationMeta {
        corpus_hash: [9; 32],
        estimator: "synthetic-conformal".to_string(),
        far: 0.01,
        frr: 0.02,
        confidence: 0.99,
        ts: 1_786_233_600,
        per_slot,
    }
}

fn profile_level_calibration() -> CalibrationMeta {
    CalibrationMeta {
        per_slot: BTreeMap::new(),
        ..calibration()
    }
}

fn slot_calibration() -> SlotCalibrationMeta {
    SlotCalibrationMeta {
        corpus_hash: [8; 32],
        estimator: "synthetic-conformal-slot".to_string(),
        far: 0.01,
        frr: 0.02,
        confidence: 0.99,
        ts: 1_786_233_600,
    }
}

fn produced_slots() -> ProducedSlots {
    [(slot(1), vec![1.0, 0.0])].into_iter().collect()
}

fn matched_slots(cos: f32) -> MatchedSlots {
    [(slot(1), vec![cos, (1.0 - cos * cos).sqrt()])]
        .into_iter()
        .collect()
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

fn assert_close(actual: f32, expected: f32) {
    assert!(
        (actual - expected).abs() <= 1.0e-5,
        "actual={actual} expected={expected}"
    );
}

fn guard_id() -> GuardId {
    GUARD_UUID.parse().expect("guard id")
}

const fn slot(value: u16) -> SlotId {
    SlotId::new(value)
}
