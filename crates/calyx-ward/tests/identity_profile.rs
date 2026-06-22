use std::collections::BTreeMap;
use std::path::Path;

use calyx_core::{AnchorKind, FixedClock, SlotId};
use calyx_ward::{
    CALYX_GUARD_IDENTITY_SLOT_NOT_REQUIRED, CalibrationMeta, GuardId, GuardPolicy, GuardProfile,
    IdentityProfile, IdentitySlotConfig, NoveltyAction, WardError,
};
use proptest::prelude::*;
use serde_json::json;

#[test]
fn identity_profile_builds_for_speaker_and_style_slots() {
    let profile = IdentityProfile::new(
        profile_template(vec![slot(8), slot(9)], true),
        identity_slots(None, None),
        matched_vecs(),
    )
    .expect("identity profile");

    assert!(profile.is_calibrated());
    assert_eq!(profile.identity_slots.len(), 2);
    assert_eq!(profile.matched_slot_cache.len(), 2);
    assert_eq!(profile.required_identity_slots(), vec![slot(8), slot(9)]);
    assert_eq!(profile.matched_slot_cache[&slot(8)], vec![1.0, 0.0]);
    assert_eq!(profile.matched_slot_cache[&slot(9)], vec![0.0, 1.0, 0.0]);

    let json = serde_json::to_string(&profile).expect("profile json");
    let decoded = serde_json::from_str::<IdentityProfile>(&json).expect("profile decode");
    assert_eq!(decoded, profile);
}

#[test]
fn anchor_identity_kinds_roundtrip_through_core_json() {
    let speaker = serde_json::to_string(&AnchorKind::SpeakerMatch).expect("speaker json");
    let style = serde_json::to_string(&AnchorKind::StyleHold).expect("style json");

    assert_eq!(speaker, "\"speaker_match\"");
    assert_eq!(style, "\"style_hold\"");
    assert_eq!(
        serde_json::from_str::<AnchorKind>(&speaker).expect("speaker decode"),
        AnchorKind::SpeakerMatch
    );
    assert_eq!(
        serde_json::from_str::<AnchorKind>(&style).expect("style decode"),
        AnchorKind::StyleHold
    );
}

#[test]
fn identity_slot_absent_from_required_slots_fails_closed() {
    let error = IdentityProfile::new(
        profile_template(vec![slot(8)], true),
        identity_slots(None, None),
        matched_vecs(),
    )
    .expect_err("slot 9 is not required");

    assert_eq!(error, WardError::IdentitySlotNotRequired { slot: slot(9) });
    assert_eq!(error.code(), CALYX_GUARD_IDENTITY_SLOT_NOT_REQUIRED);
}

#[test]
fn missing_matched_identity_vector_fails_closed() {
    let mut matched = matched_vecs();
    matched.remove(&slot(9));

    let error = IdentityProfile::new(
        profile_template(vec![slot(8), slot(9)], true),
        identity_slots(None, None),
        matched,
    )
    .expect_err("missing style matched vector");

    assert_eq!(error, WardError::MissingSlot { slot: slot(9) });
}

#[test]
fn identity_slot_config_rejects_nan_tau_override() {
    let error = IdentityProfile::new(
        profile_template(vec![slot(8), slot(9)], true),
        identity_slots(Some(f32::NAN), None),
        matched_vecs(),
    )
    .expect_err("nan tau override");

    assert!(matches!(error, WardError::InvalidCalibrationInput { .. }));
    assert!(error.to_string().contains("identity tau"));
}

#[test]
fn identity_slot_config_rejects_out_of_range_tau_override() {
    let error = IdentityProfile::new(
        profile_template(vec![slot(8), slot(9)], true),
        identity_slots(Some(-0.01), None),
        matched_vecs(),
    )
    .expect_err("negative tau override");

    assert!(matches!(error, WardError::InvalidCalibrationInput { .. }));
    assert!(error.to_string().contains("within [0, 1]"));
}

#[test]
fn inherited_identity_tau_must_stay_in_unit_interval() {
    let mut guard_profile = profile_template(vec![slot(8), slot(9)], true);
    guard_profile.tau.insert(slot(9), 1.01);

    let error = IdentityProfile::new(guard_profile, identity_slots(None, None), matched_vecs())
        .expect_err("inherited tau above one");

    assert!(matches!(error, WardError::InvalidCalibrationInput { .. }));
    assert!(error.to_string().contains("within [0, 1]"));
}

#[test]
fn identity_slot_requires_profile_tau_without_override() {
    let mut guard_profile = profile_template(vec![slot(8), slot(9)], true);
    guard_profile.tau.remove(&slot(9));

    let error = IdentityProfile::new(guard_profile, identity_slots(None, None), matched_vecs())
        .expect_err("missing style tau");

    assert!(matches!(error, WardError::InvalidCalibrationInput { .. }));
    assert!(error.to_string().contains("tau must be present"));
}

#[test]
fn every_required_slot_must_be_configured_as_identity() {
    let error = IdentityProfile::new(
        profile_template(vec![slot(8), slot(9)], true),
        vec![IdentitySlotConfig {
            slot_id: slot(8),
            anchor_kind: AnchorKind::SpeakerMatch,
            tau_override: None,
        }],
        BTreeMap::from([(slot(8), vec![1.0, 0.0])]),
    )
    .expect_err("required slot missing identity config");

    assert!(matches!(
        error,
        WardError::InvalidRequiredSlotDerivation { .. }
    ));
    assert!(
        error
            .to_string()
            .contains("required slots must match identity slots")
    );
}

#[test]
fn non_identity_anchor_kind_fails_closed() {
    let error = IdentityProfile::new(
        profile_template(vec![slot(8)], true),
        vec![IdentitySlotConfig {
            slot_id: slot(8),
            anchor_kind: AnchorKind::Reward,
            tau_override: None,
        }],
        BTreeMap::from([(slot(8), vec![1.0, 0.0])]),
    )
    .expect_err("non identity anchor");

    assert!(matches!(
        error,
        WardError::InvalidRequiredSlotDerivation { .. }
    ));
    assert!(error.to_string().contains("SpeakerMatch or StyleHold"));
}

#[test]
fn zero_matched_identity_vector_fails_closed() {
    let error = IdentityProfile::new(
        profile_template(vec![slot(8)], true),
        vec![IdentitySlotConfig {
            slot_id: slot(8),
            anchor_kind: AnchorKind::SpeakerMatch,
            tau_override: None,
        }],
        BTreeMap::from([(slot(8), vec![0.0, 0.0])]),
    )
    .expect_err("zero matched vector");

    assert!(matches!(error, WardError::InvalidCalibrationInput { .. }));
    assert!(error.to_string().contains("non-zero norm"));
}

#[test]
fn identity_profile_deserialize_revalidates_invariants() {
    let profile = IdentityProfile::new(
        profile_template(vec![slot(8), slot(9)], true),
        identity_slots(Some(0.91), Some(0.82)),
        matched_vecs(),
    )
    .expect("identity profile");
    let mut value = serde_json::to_value(&profile).expect("profile json");
    value["identity_slots"] = json!([{
        "slot_id": 8,
        "anchor_kind": "speaker_match"
    }]);

    let error = serde_json::from_value::<IdentityProfile>(value).expect_err("invalid json profile");

    assert!(
        error
            .to_string()
            .contains("required slots must match identity slots")
    );
}

#[test]
fn empty_identity_slot_set_is_allowed() {
    let profile = IdentityProfile::new(
        profile_template(Vec::new(), false),
        Vec::new(),
        BTreeMap::new(),
    )
    .expect("empty identity profile");

    assert!(!profile.is_calibrated());
    assert!(profile.identity_slots.is_empty());
    assert!(profile.matched_slot_cache.is_empty());
}

#[test]
fn tau_override_updates_guard_profile_tau() {
    let profile = IdentityProfile::new(
        profile_template(vec![slot(8), slot(9)], true),
        identity_slots(Some(0.91), Some(0.82)),
        matched_vecs(),
    )
    .expect("identity profile");

    assert_eq!(profile.guard_profile.tau_for(&slot(8)), Some(0.91));
    assert_eq!(profile.guard_profile.tau_for(&slot(9)), Some(0.82));
}

proptest! {
    #[test]
    fn identity_slot_config_roundtrips(slot_id in any::<u16>(), tau in prop::option::of(0.0f32..1.0f32)) {
        let config = IdentitySlotConfig {
            slot_id: slot(slot_id),
            anchor_kind: AnchorKind::SpeakerMatch,
            tau_override: tau,
        };

        let json = serde_json::to_string(&config).expect("config json");
        let decoded: IdentitySlotConfig = serde_json::from_str(&json).expect("config decode");

        prop_assert!(decoded.is_identity_anchor());
        prop_assert_eq!(decoded, config);
    }
}

#[test]
#[ignore = "manual FSV fixture; set CALYX_WARD_IDENTITY_FSV_DIR"]
fn issue269_identity_profile_fsv_writes_readbacks() {
    let root = std::env::var("CALYX_WARD_IDENTITY_FSV_DIR")
        .expect("CALYX_WARD_IDENTITY_FSV_DIR is required");
    std::fs::create_dir_all(&root).expect("create fsv root");

    let profile = IdentityProfile::new(
        profile_template(vec![slot(8), slot(9)], true),
        identity_slots(Some(0.91), None),
        matched_vecs(),
    )
    .expect("identity profile");
    let missing_required = IdentityProfile::new(
        profile_template(vec![slot(8)], true),
        identity_slots(None, None),
        matched_vecs(),
    )
    .expect_err("not required");
    let missing_matched = IdentityProfile::new(
        profile_template(vec![slot(8), slot(9)], true),
        identity_slots(None, None),
        BTreeMap::from([(slot(8), vec![1.0, 0.0])]),
    )
    .expect_err("missing matched");
    let nan_tau = IdentityProfile::new(
        profile_template(vec![slot(8), slot(9)], true),
        identity_slots(Some(f32::NAN), None),
        matched_vecs(),
    )
    .expect_err("nan tau");
    let negative_tau = IdentityProfile::new(
        profile_template(vec![slot(8), slot(9)], true),
        identity_slots(Some(-0.01), None),
        matched_vecs(),
    )
    .expect_err("negative tau");
    let mut inherited_bad_profile = profile_template(vec![slot(8), slot(9)], true);
    inherited_bad_profile.tau.insert(slot(9), 1.01);
    let inherited_bad_tau = IdentityProfile::new(
        inherited_bad_profile,
        identity_slots(None, None),
        matched_vecs(),
    )
    .expect_err("inherited bad tau");
    let mut missing_tau_profile = profile_template(vec![slot(8), slot(9)], true);
    missing_tau_profile.tau.remove(&slot(9));
    let missing_tau = IdentityProfile::new(
        missing_tau_profile,
        identity_slots(None, None),
        matched_vecs(),
    )
    .expect_err("missing tau");
    let missing_identity_slot = IdentityProfile::new(
        profile_template(vec![slot(8), slot(9)], true),
        vec![IdentitySlotConfig {
            slot_id: slot(8),
            anchor_kind: AnchorKind::SpeakerMatch,
            tau_override: None,
        }],
        BTreeMap::from([(slot(8), vec![1.0, 0.0])]),
    )
    .expect_err("missing identity slot");
    let non_identity_anchor = IdentityProfile::new(
        profile_template(vec![slot(8)], true),
        vec![IdentitySlotConfig {
            slot_id: slot(8),
            anchor_kind: AnchorKind::Reward,
            tau_override: None,
        }],
        BTreeMap::from([(slot(8), vec![1.0, 0.0])]),
    )
    .expect_err("non identity anchor");
    let zero_matched = IdentityProfile::new(
        profile_template(vec![slot(8)], true),
        vec![IdentitySlotConfig {
            slot_id: slot(8),
            anchor_kind: AnchorKind::SpeakerMatch,
            tau_override: None,
        }],
        BTreeMap::from([(slot(8), vec![0.0, 0.0])]),
    )
    .expect_err("zero matched");

    write_json(Path::new(&root).join("identity-profile.json"), &profile);
    write_json(
        Path::new(&root).join("anchor-kinds.json"),
        &json!({
            "speaker": AnchorKind::SpeakerMatch,
            "style": AnchorKind::StyleHold,
            "debug": ["SpeakerMatch", "StyleHold"],
        }),
    );
    write_json(
        Path::new(&root).join("identity-errors.json"),
        &json!([
            error_json(&missing_required),
            error_json(&missing_matched),
            error_json(&nan_tau),
            error_json(&negative_tau),
            error_json(&inherited_bad_tau),
            error_json(&missing_tau),
            error_json(&missing_identity_slot),
            error_json(&non_identity_anchor),
            error_json(&zero_matched),
        ]),
    );
    write_json(
        Path::new(&root).join("identity-summary.json"),
        &json!({
            "identity_slot_count": profile.identity_slots.len(),
            "matched_slot_count": profile.matched_slot_cache.len(),
            "required_identity_slots": profile.required_identity_slots(),
            "speaker_tau": profile.guard_profile.tau_for(&slot(8)),
            "style_tau": profile.guard_profile.tau_for(&slot(9)),
            "calibrated": profile.is_calibrated(),
            "identity_slot_error_code": missing_required.code(),
        }),
    );

    println!(
        "ISSUE269_IDENTITY_FSV slots={} matched={} error={}",
        profile.identity_slots.len(),
        profile.matched_slot_cache.len(),
        missing_required.code()
    );
}

fn identity_slots(speaker_tau: Option<f32>, style_tau: Option<f32>) -> Vec<IdentitySlotConfig> {
    vec![
        IdentitySlotConfig {
            slot_id: slot(8),
            anchor_kind: AnchorKind::SpeakerMatch,
            tau_override: speaker_tau,
        },
        IdentitySlotConfig {
            slot_id: slot(9),
            anchor_kind: AnchorKind::StyleHold,
            tau_override: style_tau,
        },
    ]
}

fn matched_vecs() -> BTreeMap<SlotId, Vec<f32>> {
    BTreeMap::from([(slot(8), vec![3.0, 0.0]), (slot(9), vec![0.0, 4.0, 0.0])])
}

fn profile_template(required_slots: Vec<SlotId>, calibrated: bool) -> GuardProfile {
    let mut tau = BTreeMap::new();
    tau.insert(slot(8), 0.88);
    tau.insert(slot(9), 0.76);
    GuardProfile {
        guard_id: GUARD_UUID.parse::<GuardId>().expect("guard id"),
        panel_version: 39,
        domain: "identity".to_string(),
        tau,
        required_slots,
        policy: GuardPolicy::AllRequired,
        calibration: calibrated.then(|| {
            CalibrationMeta::new(
                [9; 32],
                "identity_calibration_v1",
                0.01,
                0.0,
                0.95,
                &clock(),
            )
        }),
        novelty_action: NoveltyAction::Quarantine,
    }
}

fn error_json(error: &WardError) -> serde_json::Value {
    json!({
        "code": error.code(),
        "message": error.to_string(),
    })
}

fn write_json(path: impl AsRef<Path>, value: &impl serde::Serialize) {
    let file = std::fs::File::create(path).expect("create fsv json");
    serde_json::to_writer_pretty(file, value).expect("write fsv json");
}

fn clock() -> FixedClock {
    FixedClock::new(39_000)
}

const fn slot(value: u16) -> SlotId {
    SlotId::new(value)
}

const GUARD_UUID: &str = "018f48a4-9a79-74d2-8a5c-9ad7f6b8c101";
