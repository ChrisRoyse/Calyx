use std::collections::BTreeMap;

use calyx_core::SlotId;
use calyx_ward::{
    CalibrationMeta, GuardId, GuardPolicy, GuardProfile, GuardVerdict, MatchedSlots, NoveltyAction,
    ProducedSlots, WardError, guard_non_high_stakes as guard, guard_result,
};
use serde_json::json;

const GUARD_UUID: &str = "018f48a4-9a79-74d2-8a5c-9ad7f6b8c101";

#[test]
fn fsv_per_slot_verdict_readback() {
    let profile = two_slot_profile(GuardPolicy::AllRequired, [(slot(1), 0.72), (slot(2), 0.65)]);
    let produced = slot_vectors(&[(slot(1), vec![1.0, 0.0]), (slot(2), vec![1.0, 0.0])]);
    let matched = slot_vectors(&[(slot(1), cos_vector(0.80)), (slot(2), cos_vector(0.70))]);

    let verdict = guard(&profile, &produced, &matched).expect("guard succeeds");
    let json = serde_json::to_string_pretty(&verdict).expect("verdict json");

    println!("FSV_PH37_VERDICT_DEBUG {verdict:?}");
    println!("FSV_PH37_VERDICT_JSON {json}");
    assert!(verdict.overall_pass);
    assert_eq!(verdict.per_slot.len(), 2);
    assert!(verdict.per_slot.iter().all(|slot| slot.cos.is_finite()));
    assert!(
        verdict
            .per_slot
            .iter()
            .all(|slot| (-1.0..=1.0).contains(&slot.cos))
    );
    assert!(verdict.per_slot.iter().all(|slot| slot.pass));
}

#[test]
fn fsv_average_passing_slot_failing_rejected() {
    let (profile, produced, matched) = scenario(GuardPolicy::AllRequired, &[0.95, 0.45]);

    let verdict = guard(&profile, &produced, &matched).expect("guard succeeds");

    println!(
        "FSV_PH37_AVERAGE_ATTACK overall_pass={} average_would_pass={}",
        verdict.overall_pass,
        average_cosine_would_pass(&verdict)
    );
    assert!(!verdict.overall_pass);
    assert!(average_cosine_would_pass(&verdict));
    assert_eq!(failing_slots(&verdict), vec![2]);
}

#[test]
fn fsv_ood_code_emitted() {
    let (profile, produced, matched) = scenario(GuardPolicy::AllRequired, &[0.95, 0.45]);

    let error = guard_result(&profile, &produced, &matched).expect_err("attack is OOD");
    let formatted = error.to_string();

    println!("FSV_PH37_OOD {formatted}");
    assert_eq!(error.code(), "CALYX_GUARD_OOD");
    assert!(formatted.contains("CALYX_GUARD_OOD"));
    assert!(formatted.contains("slot 2"));
}

#[test]
fn fsv_no_flatten_source_check() {
    let source = guard_source();
    let markers = aggregate_vector_gate_markers(&source);
    let line_count = source.lines().count();

    println!("FSV_PH37_SOURCE line_count={line_count} markers={markers:?}");
    assert!(source.contains("INVARIANT A3"));
    assert!(markers.is_empty(), "unexpected source markers: {markers:?}");
    assert!(line_count <= 500);
}

#[test]
fn fsv_guard_profile_serde_roundtrip() {
    let profile = calibrated_profile();
    let json = serde_json::to_string_pretty(&profile).expect("profile json");
    let decoded: GuardProfile = serde_json::from_str(&json).expect("profile decode");

    println!("FSV_PH37_PROFILE_JSON {json}");
    assert_eq!(decoded, profile);
    assert!(json.contains("calibration"));
    assert!(json.contains("Quarantine"));
}

#[test]
fn invalid_vector_tau_zero_still_fails_closed() {
    let profile = one_slot_profile(GuardPolicy::AllRequired, slot(1), 0.0);
    let produced = slot_vectors(&[(slot(1), vec![0.0, 0.0])]);
    let matched = slot_vectors(&[(slot(1), vec![1.0, 0.0])]);

    let verdict = guard(&profile, &produced, &matched).expect("guard succeeds");

    assert!(!verdict.overall_pass);
    assert_eq!(verdict.per_slot[0].cos, 0.0);
    assert!(!verdict.per_slot[0].pass);
}

#[test]
#[ignore = "manual FSV fixture; set CALYX_WARD_PH37_FSV_DIR"]
fn guard_ph37_fsv_fixture_writes_readback_artifacts() {
    let root =
        std::env::var("CALYX_WARD_PH37_FSV_DIR").expect("CALYX_WARD_PH37_FSV_DIR is required");
    std::fs::create_dir_all(&root).expect("create fsv root");

    let profile = two_slot_profile(GuardPolicy::AllRequired, [(slot(1), 0.72), (slot(2), 0.65)]);
    let produced = slot_vectors(&[(slot(1), vec![1.0, 0.0]), (slot(2), vec![1.0, 0.0])]);
    let matched = slot_vectors(&[(slot(1), cos_vector(0.80)), (slot(2), cos_vector(0.70))]);
    let per_slot = guard(&profile, &produced, &matched).expect("per-slot verdict");

    let (attack_profile, attack_produced, attack_matched) =
        scenario(GuardPolicy::AllRequired, &[0.95, 0.45]);
    let attack = guard(&attack_profile, &attack_produced, &attack_matched).expect("attack verdict");
    let ood =
        guard_result(&attack_profile, &attack_produced, &attack_matched).expect_err("ood error");

    let invalid = guard(
        &one_slot_profile(GuardPolicy::AllRequired, slot(1), 0.0),
        &slot_vectors(&[(slot(1), vec![0.0, 0.0])]),
        &slot_vectors(&[(slot(1), vec![1.0, 0.0])]),
    )
    .expect("invalid vector verdict");

    let source = guard_source();
    let profile = calibrated_profile();
    let profile_json = serde_json::to_string_pretty(&profile).expect("profile json");
    let roundtrip: GuardProfile = serde_json::from_str(&profile_json).expect("profile roundtrip");

    write_json(&root, "per-slot-verdict.json", &per_slot);
    write_json(
        &root,
        "average-attack-verdict.json",
        &verdict_with_average(&attack),
    );
    write_json(&root, "ood-error.json", &error_json(&ood));
    write_json(&root, "invalid-vector-verdict.json", &invalid);
    write_json(
        &root,
        "source-readback.json",
        &json!({
            "line_count": source.lines().count(),
            "contains_a3_invariant": source.contains("INVARIANT A3"),
            "aggregate_vector_gate_markers": aggregate_vector_gate_markers(&source),
        }),
    );
    write_json(
        &root,
        "profile-roundtrip.json",
        &json!({
            "profile": roundtrip,
            "roundtrip_equal": roundtrip == profile,
        }),
    );

    println!(
        "FSV_PH37 per_slot_overall={} attack_overall={} average_would_pass={} ood_code={} invalid_pass={}",
        per_slot.overall_pass,
        attack.overall_pass,
        average_cosine_would_pass(&attack),
        ood.code(),
        invalid.overall_pass
    );
}

fn verdict_with_average(verdict: &GuardVerdict) -> serde_json::Value {
    json!({
        "verdict": verdict,
        "average_would_pass": average_cosine_would_pass(verdict),
        "failing_slots": failing_slots(verdict),
    })
}

fn average_cosine_would_pass(verdict: &GuardVerdict) -> bool {
    if verdict.per_slot.is_empty() {
        return true;
    }
    let mean_cos = mean(verdict.per_slot.iter().map(|slot| slot.cos));
    let mean_tau = mean(verdict.per_slot.iter().map(|slot| slot.tau));
    mean_cos >= mean_tau
}

fn mean(values: impl Iterator<Item = f32>) -> f32 {
    let mut sum = 0.0;
    let mut count = 0_u32;
    for value in values {
        sum += value;
        count += 1;
    }
    if count == 0 { 0.0 } else { sum / count as f32 }
}

fn failing_slots(verdict: &GuardVerdict) -> Vec<u16> {
    verdict
        .per_slot
        .iter()
        .filter(|slot| !slot.pass)
        .map(|slot| slot.slot.get())
        .collect()
}

fn aggregate_vector_gate_markers(source: &str) -> Vec<String> {
    let markers = [
        "flatten",
        "concat",
        "extend_from_slice",
        ".append(",
        ".extend(",
        ".chain(",
        "flat_map",
        "collect::<Vec",
    ];
    source
        .lines()
        .enumerate()
        .filter_map(|(index, line)| {
            let trimmed = line.trim_start();
            if trimmed.is_empty() || trimmed.starts_with("//") {
                return None;
            }
            markers
                .iter()
                .find(|marker| trimmed.contains(**marker))
                .map(|marker| format!("{}:{marker}", index + 1))
        })
        .collect()
}

fn guard_source() -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src")
        .join("guard.rs");
    std::fs::read_to_string(path).expect("read guard.rs")
}

fn error_json(error: &WardError) -> serde_json::Value {
    json!({
        "code": error.code(),
        "message": error.to_string(),
    })
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

fn two_slot_profile(policy: GuardPolicy, entries: [(SlotId, f32); 2]) -> GuardProfile {
    let mut tau = BTreeMap::new();
    let mut required_slots = Vec::new();
    for (slot, value) in entries {
        tau.insert(slot, value);
        required_slots.push(slot);
    }
    GuardProfile {
        guard_id: guard_id(),
        panel_version: 42,
        domain: "synthetic".to_string(),
        tau,
        required_slots,
        policy,
        calibration: None,
        novelty_action: NoveltyAction::NewRegion,
    }
}

fn one_slot_profile(policy: GuardPolicy, slot: SlotId, tau_value: f32) -> GuardProfile {
    GuardProfile {
        guard_id: guard_id(),
        panel_version: 42,
        domain: "synthetic".to_string(),
        tau: [(slot, tau_value)].into_iter().collect(),
        required_slots: vec![slot],
        policy,
        calibration: None,
        novelty_action: NoveltyAction::Quarantine,
    }
}

fn calibrated_profile() -> GuardProfile {
    let mut profile =
        two_slot_profile(GuardPolicy::AllRequired, [(slot(1), 0.72), (slot(2), 0.65)]);
    profile.calibration = Some(CalibrationMeta {
        corpus_hash: [7; 32],
        estimator: "synthetic-fixed".to_string(),
        far: 0.01,
        frr: 0.05,
        confidence: 0.99,
        ts: 1_776_742_400,
        per_slot: BTreeMap::new(),
    });
    profile.novelty_action = NoveltyAction::Quarantine;
    profile
}

fn slot_vectors(entries: &[(SlotId, Vec<f32>)]) -> BTreeMap<SlotId, Vec<f32>> {
    entries.iter().cloned().collect()
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
