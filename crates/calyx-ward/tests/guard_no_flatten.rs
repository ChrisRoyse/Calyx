use std::collections::BTreeMap;

use calyx_core::SlotId;
use calyx_ward::{
    GuardId, GuardPolicy, GuardProfile, GuardVerdict, MatchedSlots, NoveltyAction, ProducedSlots,
    WardError, guard_non_high_stakes as guard, guard_result,
};
use proptest::prelude::*;
use serde_json::json;

const GUARD_UUID: &str = "018f48a4-9a79-74d2-8a5c-9ad7f6b8c101";

#[test]
fn average_passing_slot_failing_attack_is_rejected() {
    let (profile, produced, matched) = scenario(GuardPolicy::AllRequired, &[0.95, 0.45]);

    let verdict = guard(&profile, &produced, &matched).expect("guard succeeds");

    assert!(!verdict.overall_pass);
    assert!(average_cosine_would_pass(&verdict));
    assert_eq!(verdict.per_slot[0].slot, slot(1));
    assert!(verdict.per_slot[0].pass);
    assert_eq!(verdict.per_slot[1].slot, slot(2));
    assert!(!verdict.per_slot[1].pass);
}

#[test]
fn average_passing_slot_failing_attack_returns_ood_from_guard_result() {
    let (profile, produced, matched) = scenario(GuardPolicy::AllRequired, &[0.95, 0.45]);

    let error = guard_result(&profile, &produced, &matched).expect_err("attack is OOD");

    assert_eq!(
        error,
        WardError::Ood {
            guard_id: guard_id(),
            failing: vec![calyx_ward::SlotVerdict {
                slot: slot(2),
                cos: 0.45,
                tau: 0.7,
                pass: false,
            }]
        }
    );
    assert!(error.to_string().contains("CALYX_GUARD_OOD"));
}

#[test]
fn average_above_tau_still_fails_allrequired_but_can_pass_kofn_one() {
    let (_, produced, matched) = scenario(GuardPolicy::AllRequired, &[0.93, 0.60, 0.60]);
    let all_required = profile(GuardPolicy::AllRequired, 3);
    let kofn_one = profile(GuardPolicy::KofN { k: 1 }, 3);

    let all_required_verdict =
        guard(&all_required, &produced, &matched).expect("allrequired verdict");
    let kofn_verdict = guard(&kofn_one, &produced, &matched).expect("kofn verdict");

    assert!(average_cosine_would_pass(&all_required_verdict));
    assert!(!all_required_verdict.overall_pass);
    assert_eq!(all_required_verdict.failing_slots().len(), 2);
    assert!(kofn_verdict.overall_pass);
}

#[test]
fn identical_vectors_all_pass() {
    let (profile, produced, matched) = scenario(GuardPolicy::AllRequired, &[1.0, 1.0, 1.0]);

    let verdict = guard(&profile, &produced, &matched).expect("guard succeeds");

    assert!(verdict.overall_pass);
    assert!(verdict.per_slot.iter().all(|slot| slot.pass));
}

#[test]
fn one_zero_cos_slot_fails_regardless_of_high_other_slots() {
    let (profile, produced, matched) = scenario(GuardPolicy::AllRequired, &[1.0, 1.0, 0.0]);

    let verdict = guard(&profile, &produced, &matched).expect("guard succeeds");

    assert!(!verdict.overall_pass);
    assert_eq!(verdict.failing_slots().len(), 1);
    assert_eq!(verdict.failing_slots()[0].slot, slot(3));
}

#[test]
fn empty_average_is_safe_and_non_panicking() {
    let verdict = GuardVerdict {
        guard_id: guard_id(),
        overall_pass: true,
        provisional: false,
        per_slot: Vec::new(),
        action: None,
    };

    assert!(average_cosine_would_pass(&verdict));
}

#[test]
fn source_readback_has_no_aggregate_vector_gate_markers() {
    let source = guard_source();
    let findings = aggregate_vector_gate_markers(&source);

    assert!(source.contains("INVARIANT A3"));
    assert!(
        findings.is_empty(),
        "unexpected source markers: {findings:?}"
    );
}

proptest! {
    #[test]
    fn allrequired_fails_when_any_slot_below_tau(
        first in 0.0f32..1.0,
        second in 0.0f32..1.0,
        third in 0.0f32..1.0,
    ) {
        let scores = [first, second, third];
        prop_assume!(scores.iter().any(|score| *score < 0.7));
        let (profile, produced, matched) = scenario(GuardPolicy::AllRequired, &scores);

        let verdict = guard(&profile, &produced, &matched).expect("guard succeeds");

        prop_assert!(!verdict.overall_pass);
    }
}

#[test]
#[ignore = "manual FSV fixture; set CALYX_WARD_NO_FLATTEN_FSV_DIR"]
fn guard_no_flatten_fsv_fixture_writes_readback_artifacts() {
    let root = std::env::var("CALYX_WARD_NO_FLATTEN_FSV_DIR")
        .expect("CALYX_WARD_NO_FLATTEN_FSV_DIR is required");
    std::fs::create_dir_all(&root).expect("create fsv root");

    let (attack_profile, attack_produced, attack_matched) =
        scenario(GuardPolicy::AllRequired, &[0.95, 0.45]);
    let attack = guard(&attack_profile, &attack_produced, &attack_matched).expect("attack verdict");
    let attack_ood =
        guard_result(&attack_profile, &attack_produced, &attack_matched).expect_err("attack OOD");
    let (_, three_produced, three_matched) =
        scenario(GuardPolicy::AllRequired, &[0.93, 0.60, 0.60]);
    let all_required = guard(
        &profile(GuardPolicy::AllRequired, 3),
        &three_produced,
        &three_matched,
    )
    .expect("allrequired verdict");
    let kofn_one = guard(
        &profile(GuardPolicy::KofN { k: 1 }, 3),
        &three_produced,
        &three_matched,
    )
    .expect("kofn verdict");
    let source = guard_source();
    let markers = aggregate_vector_gate_markers(&source);

    write_json(
        &root,
        "average-attack-verdict.json",
        &verdict_with_average(&attack),
    );
    write_json(
        &root,
        "average-attack-ood-error.json",
        &error_json(&attack_ood),
    );
    write_json(
        &root,
        "three-slot-allrequired-verdict.json",
        &verdict_with_average(&all_required),
    );
    write_json(
        &root,
        "three-slot-kofn-one-verdict.json",
        &verdict_with_average(&kofn_one),
    );
    write_json(
        &root,
        "source-readback.json",
        &json!({
            "line_count": source.lines().count(),
            "contains_a3_invariant": source.contains("INVARIANT A3"),
            "aggregate_vector_gate_markers": markers,
        }),
    );

    println!(
        "FSV_NO_FLATTEN attack_overall={} average_would_pass={}",
        attack.overall_pass,
        average_cosine_would_pass(&attack)
    );
    println!(
        "FSV_NO_FLATTEN allrequired={} kofn_one={}",
        all_required.overall_pass, kofn_one.overall_pass
    );
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

fn verdict_with_average(verdict: &GuardVerdict) -> serde_json::Value {
    json!({
        "verdict": verdict,
        "average_would_pass": average_cosine_would_pass(verdict),
        "failing_slots": verdict
            .per_slot
            .iter()
            .filter(|slot| !slot.pass)
            .map(|slot| slot.slot.get())
            .collect::<Vec<_>>(),
    })
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
    let profile = profile(policy, cos_scores.len());
    let mut produced = BTreeMap::new();
    let mut matched = BTreeMap::new();
    for (index, cos) in cos_scores.iter().copied().enumerate() {
        let slot = slot((index + 1) as u16);
        produced.insert(slot, vec![1.0, 0.0]);
        matched.insert(slot, cos_vector(cos));
    }
    (profile, produced, matched)
}

fn profile(policy: GuardPolicy, n_slots: usize) -> GuardProfile {
    let mut tau = BTreeMap::new();
    let mut required_slots = Vec::new();
    for index in 0..n_slots {
        let slot = slot((index + 1) as u16);
        tau.insert(slot, 0.7);
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
        novelty_action: NoveltyAction::Quarantine,
    }
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
