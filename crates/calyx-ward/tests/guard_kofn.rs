use std::collections::BTreeMap;

use calyx_core::SlotId;
use calyx_ward::{
    GuardId, GuardPolicy, GuardProfile, MatchedSlots, NoveltyAction, ProducedSlots, WardError,
    guard_non_high_stakes as guard, guard_result,
};
use serde_json::json;

const GUARD_UUID: &str = "018f48a4-9a79-74d2-8a5c-9ad7f6b8c101";

#[test]
fn kofn_two_of_three_passes_with_one_failed_slot_detail() {
    let (profile, produced, matched) = scenario(GuardPolicy::KofN { k: 2 }, &[0.8, 0.4, 0.9]);

    let verdict = guard(&profile, &produced, &matched).expect("guard succeeds");

    assert!(verdict.overall_pass);
    assert_eq!(verdict.action, None);
    assert_eq!(verdict.per_slot.len(), 3);
    assert_eq!(verdict.failing_slots().len(), 1);
    assert_eq!(verdict.failing_slots()[0].slot, slot(2));
}

#[test]
fn kofn_three_of_three_fails_and_guard_result_returns_ood() {
    let (profile, produced, matched) = scenario(GuardPolicy::KofN { k: 3 }, &[0.8, 0.4, 0.9]);

    let verdict = guard(&profile, &produced, &matched).expect("guard succeeds");
    let error = guard_result(&profile, &produced, &matched).expect_err("ood error");

    assert!(!verdict.overall_pass);
    assert_eq!(verdict.action, Some(NoveltyAction::Quarantine));
    assert_eq!(
        error,
        WardError::Ood {
            guard_id: guard_id(),
            failing: vec![verdict.per_slot[1].clone()]
        }
    );
    assert!(error.to_string().contains("CALYX_GUARD_OOD"));
}

#[test]
fn kofn_one_of_three_fails_when_all_slots_fail() {
    let (profile, produced, matched) = scenario(GuardPolicy::KofN { k: 1 }, &[0.1, 0.2, 0.3]);

    let verdict = guard(&profile, &produced, &matched).expect("guard succeeds");

    assert!(!verdict.overall_pass);
    assert_eq!(verdict.failing_slots().len(), 3);
}

#[test]
fn kofn_zero_fails_inert_profile_even_when_slots_fail() {
    let (profile, produced, matched) = scenario(GuardPolicy::KofN { k: 0 }, &[0.1, 0.2, 0.3]);

    let error = guard(&profile, &produced, &matched).expect_err("inert profile");

    assert_eq!(
        error,
        WardError::InertProfile {
            guard_id: guard_id(),
            reason: "kofn_zero",
        }
    );
    assert_eq!(error.code(), "CALYX_GUARD_INERT_PROFILE");
}

#[test]
fn kofn_larger_than_required_count_fails_closed() {
    let (profile, produced, matched) = scenario(GuardPolicy::KofN { k: 4 }, &[0.8, 0.8, 0.8]);

    let error = guard(&profile, &produced, &matched).expect_err("policy violation");
    let formatted = error.to_string();

    assert_eq!(
        error,
        WardError::PolicyViolation {
            k: 4,
            n_required: 3
        }
    );
    assert!(formatted.contains("CALYX_GUARD_POLICY_VIOLATION"));
    assert!(formatted.contains("k=4"));
    assert!(formatted.contains("n_required=3"));
}

#[test]
fn kofn_boundary_equal_tau_passes() {
    let (profile, produced, matched) = scenario(GuardPolicy::KofN { k: 1 }, &[0.7]);

    let verdict = guard(&profile, &produced, &matched).expect("guard succeeds");

    assert!(verdict.overall_pass);
    assert!(verdict.per_slot[0].pass);
}

#[test]
#[ignore = "manual FSV fixture; set CALYX_WARD_KOFN_FSV_DIR"]
fn guard_kofn_fsv_fixture_writes_readback_artifacts() {
    let root =
        std::env::var("CALYX_WARD_KOFN_FSV_DIR").expect("CALYX_WARD_KOFN_FSV_DIR is required");
    std::fs::create_dir_all(&root).expect("create fsv root");

    let (k2_profile, produced, matched) = scenario(GuardPolicy::KofN { k: 2 }, &[0.8, 0.4, 0.9]);
    let (k3_profile, _, _) = scenario(GuardPolicy::KofN { k: 3 }, &[0.8, 0.4, 0.9]);
    let (k0_profile, k0_produced, k0_matched) =
        scenario(GuardPolicy::KofN { k: 0 }, &[0.1, 0.2, 0.3]);
    let (k4_profile, k4_produced, k4_matched) =
        scenario(GuardPolicy::KofN { k: 4 }, &[0.8, 0.8, 0.8]);

    let k2 = guard(&k2_profile, &produced, &matched).expect("k2 pass verdict");
    let k3 = guard(&k3_profile, &produced, &matched).expect("k3 fail verdict");
    let k0 = guard(&k0_profile, &k0_produced, &k0_matched).expect_err("k0 inert profile");
    let ood = guard_result(&k3_profile, &produced, &matched).expect_err("ood error");
    let policy = guard(&k4_profile, &k4_produced, &k4_matched).expect_err("policy violation");

    write_json(&root, "kofn-k2-pass-verdict.json", &k2);
    write_json(&root, "kofn-k3-fail-verdict.json", &k3);
    write_json(&root, "kofn-k0-inert-error.json", &error_json(&k0));
    write_json(&root, "guard-result-ood-error.json", &error_json(&ood));
    write_json(&root, "policy-violation-error.json", &error_json(&policy));

    println!(
        "FSV_KOFN k2_overall={} k3_overall={} k0_code={}",
        k2.overall_pass,
        k3.overall_pass,
        k0.code()
    );
    println!("FSV_KOFN_POLICY {}", policy);
    println!("FSV_KOFN_OOD {}", ood);
}

fn scenario(
    policy: GuardPolicy,
    cos_scores: &[f32],
) -> (GuardProfile, ProducedSlots, MatchedSlots) {
    let mut tau = BTreeMap::new();
    let mut required_slots = Vec::new();
    let mut produced = BTreeMap::new();
    let mut matched = BTreeMap::new();
    for (index, cos) in cos_scores.iter().copied().enumerate() {
        let slot = slot((index + 1) as u16);
        tau.insert(slot, 0.7);
        required_slots.push(slot);
        produced.insert(slot, vec![1.0, 0.0]);
        matched.insert(slot, cos_vector(cos));
    }
    (
        GuardProfile {
            guard_id: guard_id(),
            panel_version: 42,
            domain: "synthetic".to_string(),
            tau,
            required_slots,
            policy,
            calibration: None,
            novelty_action: NoveltyAction::Quarantine,
        },
        produced,
        matched,
    )
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

fn cos_vector(cos: f32) -> Vec<f32> {
    vec![cos, (1.0 - cos * cos).sqrt()]
}

fn guard_id() -> GuardId {
    GUARD_UUID.parse().expect("guard id")
}

const fn slot(value: u16) -> SlotId {
    SlotId::new(value)
}
