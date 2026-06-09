//! Per-slot Ward guard math.

use std::collections::BTreeMap;

use calyx_core::SlotId;
use calyx_forge::{Backend, CpuBackend};

use crate::error::WardError;
use crate::profile::{GuardPolicy, GuardProfile};
use crate::verdict::{GuardVerdict, SlotVerdict};

pub const DEFAULT_TAU: f32 = 0.7;

pub type ProducedSlots = BTreeMap<SlotId, Vec<f32>>;
pub type MatchedSlots = BTreeMap<SlotId, Vec<f32>>;

/// Evaluates every required slot independently under `GuardPolicy::AllRequired`.
///
/// Missing required slots fail closed with `WardError::MissingSlot`. Slots with
/// invalid vectors produce a failed slot verdict instead of panicking, preserving
/// the full decomposition for callers and FSV readback.
pub fn guard(
    profile: &GuardProfile,
    produced: &ProducedSlots,
    matched: &MatchedSlots,
) -> Result<GuardVerdict, WardError> {
    if let GuardPolicy::KofN { k } = profile.policy {
        return Err(WardError::PolicyViolation {
            k,
            n_required: required_slots(profile).len(),
        });
    }

    let backend = CpuBackend::new();
    let mut per_slot = Vec::new();
    for slot in required_slots(profile) {
        let produced_vec = produced.get(&slot).ok_or(WardError::MissingSlot { slot })?;
        let matched_vec = matched.get(&slot).ok_or(WardError::MissingSlot { slot })?;
        let tau = profile.tau_for(&slot).unwrap_or(DEFAULT_TAU);
        let cos = slot_cosine(&backend, produced_vec, matched_vec).unwrap_or(0.0);
        let pass = cos >= tau;
        per_slot.push(SlotVerdict {
            slot,
            cos,
            tau,
            pass,
        });
    }

    let overall_pass = per_slot.iter().all(|slot| slot.pass);
    let action = (!overall_pass).then(|| profile.novelty_action.clone());
    Ok(GuardVerdict {
        guard_id: profile.guard_id,
        overall_pass,
        per_slot,
        action,
    })
}

fn required_slots(profile: &GuardProfile) -> Vec<SlotId> {
    let mut slots = profile.required_slots.clone();
    slots.sort_unstable();
    slots.dedup();
    slots
}

fn slot_cosine(backend: &CpuBackend, produced: &[f32], matched: &[f32]) -> Option<f32> {
    if produced.len() != matched.len() {
        return None;
    }
    let dim = produced.len();
    let produced = normalized(produced)?;
    let matched = normalized(matched)?;
    let mut out = [0.0_f32; 1];
    backend
        .cosine(&produced, &matched, dim, &mut out)
        .ok()
        .map(|_| out[0])
        .filter(|cos| cos.is_finite())
}

fn normalized(values: &[f32]) -> Option<Vec<f32>> {
    if values.is_empty() || values.iter().any(|value| !value.is_finite()) {
        return None;
    }
    let norm = values.iter().map(|value| value * value).sum::<f32>().sqrt();
    if !norm.is_finite() || norm <= 0.0 {
        return None;
    }
    Some(values.iter().map(|value| value / norm).collect())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use calyx_core::SlotId;
    use proptest::prelude::*;
    use serde_json::json;

    use super::*;
    use crate::{GuardId, NoveltyAction};

    const GUARD_UUID: &str = "018f48a4-9a79-74d2-8a5c-9ad7f6b8c101";

    #[test]
    fn all_required_passes_when_every_required_slot_meets_tau() {
        let profile = sample_profile(vec![(slot(2), 0.70), (slot(1), 0.70)]);
        let produced = slot_vectors(&[(slot(1), vec![1.0, 0.0]), (slot(2), vec![1.0, 0.0])]);
        let matched = slot_vectors(&[(slot(1), cos_vector(0.90)), (slot(2), cos_vector(0.80))]);

        let verdict = guard(&profile, &produced, &matched).expect("guard succeeds");

        assert!(verdict.overall_pass);
        assert_eq!(verdict.action, None);
        assert_eq!(verdict.per_slot.len(), 2);
        assert_eq!(verdict.per_slot[0].slot, slot(1));
        assert_eq!(verdict.per_slot[1].slot, slot(2));
        assert!(verdict.per_slot.iter().all(|slot| slot.pass));
    }

    #[test]
    fn all_required_fails_single_slot_below_tau_with_full_breakdown() {
        let profile = sample_profile(vec![(slot(1), 0.70), (slot(2), 0.70)]);
        let produced = slot_vectors(&[(slot(1), vec![1.0, 0.0]), (slot(2), vec![1.0, 0.0])]);
        let matched = slot_vectors(&[(slot(1), cos_vector(0.90)), (slot(2), cos_vector(0.55))]);

        let verdict = guard(&profile, &produced, &matched).expect("guard succeeds");
        let failing = verdict.failing_slots();

        assert!(!verdict.overall_pass);
        assert_eq!(verdict.action, Some(NoveltyAction::Quarantine));
        assert_eq!(failing.len(), 1);
        assert_eq!(failing[0].slot, slot(2));
        assert_close(failing[0].cos, 0.55);
        assert_close(failing[0].tau, 0.70);
    }

    #[test]
    fn boundary_cos_equal_tau_passes() {
        let profile = sample_profile(vec![(slot(1), 1.0)]);
        let produced = slot_vectors(&[(slot(1), vec![1.0, 0.0])]);
        let matched = slot_vectors(&[(slot(1), vec![1.0, 0.0])]);

        let verdict = guard(&profile, &produced, &matched).expect("guard succeeds");

        assert!(verdict.overall_pass);
        assert!(verdict.per_slot[0].pass);
        assert_close(verdict.per_slot[0].cos, 1.0);
    }

    #[test]
    fn absent_tau_uses_default_threshold() {
        let mut profile = sample_profile(vec![]);
        profile.required_slots = vec![slot(1)];
        let produced = slot_vectors(&[(slot(1), vec![1.0, 0.0])]);
        let matched = slot_vectors(&[(slot(1), cos_vector(0.69))]);

        let verdict = guard(&profile, &produced, &matched).expect("guard succeeds");

        assert!(!verdict.overall_pass);
        assert_close(verdict.per_slot[0].tau, DEFAULT_TAU);
    }

    #[test]
    fn empty_required_slots_passes_without_action() {
        let profile = sample_profile(vec![]);
        let produced = ProducedSlots::new();
        let matched = MatchedSlots::new();

        let verdict = guard(&profile, &produced, &matched).expect("guard succeeds");

        assert!(verdict.overall_pass);
        assert!(verdict.per_slot.is_empty());
        assert_eq!(verdict.action, None);
    }

    #[test]
    fn zero_vector_returns_failed_verdict_without_panic() {
        let profile = sample_profile(vec![(slot(1), 0.70)]);
        let produced = slot_vectors(&[(slot(1), vec![0.0, 0.0])]);
        let matched = slot_vectors(&[(slot(1), vec![1.0, 0.0])]);

        let verdict = guard(&profile, &produced, &matched).expect("guard succeeds");

        assert!(!verdict.overall_pass);
        assert_eq!(verdict.action, Some(NoveltyAction::Quarantine));
        assert_eq!(verdict.per_slot[0].cos, 0.0);
        assert!(!verdict.per_slot[0].pass);
    }

    #[test]
    fn shape_mismatch_returns_failed_verdict_without_panic() {
        let profile = sample_profile(vec![(slot(1), 0.70)]);
        let produced = slot_vectors(&[(slot(1), vec![1.0, 0.0])]);
        let matched = slot_vectors(&[(slot(1), vec![1.0])]);

        let verdict = guard(&profile, &produced, &matched).expect("guard succeeds");

        assert!(!verdict.overall_pass);
        assert_eq!(verdict.per_slot[0].cos, 0.0);
    }

    #[test]
    fn missing_produced_slot_fails_closed() {
        let profile = sample_profile(vec![(slot(1), 0.70)]);
        let produced = ProducedSlots::new();
        let matched = slot_vectors(&[(slot(1), vec![1.0, 0.0])]);

        let error = guard(&profile, &produced, &matched).expect_err("missing slot");

        assert_eq!(error, WardError::MissingSlot { slot: slot(1) });
    }

    #[test]
    fn missing_matched_slot_fails_closed() {
        let profile = sample_profile(vec![(slot(1), 0.70)]);
        let produced = slot_vectors(&[(slot(1), vec![1.0, 0.0])]);
        let matched = MatchedSlots::new();

        let error = guard(&profile, &produced, &matched).expect_err("missing slot");

        assert_eq!(error, WardError::MissingSlot { slot: slot(1) });
    }

    #[test]
    fn kofn_policy_is_reserved_for_t04() {
        let mut profile = sample_profile(vec![(slot(1), 0.70)]);
        profile.policy = GuardPolicy::KofN { k: 1 };
        let produced = slot_vectors(&[(slot(1), vec![1.0, 0.0])]);
        let matched = slot_vectors(&[(slot(1), vec![1.0, 0.0])]);

        let error = guard(&profile, &produced, &matched).expect_err("reserved policy");

        assert_eq!(
            error,
            WardError::PolicyViolation {
                k: 1,
                n_required: 1
            }
        );
    }

    proptest! {
        #[test]
        fn verdict_matches_cosine_threshold(
            ax in -1.0f32..1.0,
            ay in -1.0f32..1.0,
            bx in -1.0f32..1.0,
            by in -1.0f32..1.0,
            tau in 0.0f32..1.0,
        ) {
            let a = [ax, ay];
            let b = [bx, by];
            prop_assume!(norm(&a) > 1.0e-6);
            prop_assume!(norm(&b) > 1.0e-6);
            let expected_cos = manual_cos(&a, &b);
            prop_assume!((expected_cos - tau).abs() > 1.0e-5);

            let profile = sample_profile(vec![(slot(1), tau)]);
            let produced = slot_vectors(&[(slot(1), a.to_vec())]);
            let matched = slot_vectors(&[(slot(1), b.to_vec())]);

            let verdict = guard(&profile, &produced, &matched).expect("guard succeeds");

            prop_assert_eq!(verdict.per_slot[0].pass, expected_cos >= tau);
        }
    }

    #[test]
    #[ignore = "manual aiwonder FSV fixture; set CALYX_WARD_GUARD_FSV_DIR"]
    fn guard_allrequired_fsv_fixture_writes_readback_artifacts() {
        let root = std::env::var("CALYX_WARD_GUARD_FSV_DIR")
            .expect("CALYX_WARD_GUARD_FSV_DIR is required");
        std::fs::create_dir_all(&root).expect("create fsv root");

        let fail_profile = sample_profile(vec![(slot(1), 0.70), (slot(2), 0.70)]);
        let produced = slot_vectors(&[(slot(1), vec![1.0, 0.0]), (slot(2), vec![1.0, 0.0])]);
        let fail_matched =
            slot_vectors(&[(slot(1), cos_vector(0.90)), (slot(2), cos_vector(0.55))]);
        let pass_matched =
            slot_vectors(&[(slot(1), cos_vector(0.90)), (slot(2), cos_vector(0.80))]);
        let fail = guard(&fail_profile, &produced, &fail_matched).expect("fail verdict");
        let pass = guard(&fail_profile, &produced, &pass_matched).expect("pass verdict");
        let empty = guard(
            &sample_profile(vec![]),
            &ProducedSlots::new(),
            &MatchedSlots::new(),
        )
        .expect("empty verdict");
        let zero = guard(
            &sample_profile(vec![(slot(1), 0.70)]),
            &slot_vectors(&[(slot(1), vec![0.0, 0.0])]),
            &slot_vectors(&[(slot(1), vec![1.0, 0.0])]),
        )
        .expect("zero verdict");
        let missing = guard(
            &sample_profile(vec![(slot(3), 0.70)]),
            &ProducedSlots::new(),
            &MatchedSlots::new(),
        )
        .expect_err("missing slot");

        write_json(&root, "allrequired-fail-verdict.json", &fail);
        write_json(&root, "allrequired-pass-verdict.json", &pass);
        write_json(&root, "edge-empty-required-verdict.json", &empty);
        write_json(&root, "edge-zero-vector-verdict.json", &zero);
        write_json(
            &root,
            "missing-slot-error.json",
            &json!({
                "code": missing.code(),
                "message": missing.to_string(),
            }),
        );

        println!(
            "FSV_GUARD_FAIL overall_pass={} failing_slots={}",
            fail.overall_pass,
            fail.failing_slots().len()
        );
        for detail in fail.all_slot_details() {
            println!(
                "FSV_SLOT slot={} cos={:.6} tau={:.6} pass={}",
                detail.slot, detail.cos, detail.tau, detail.pass
            );
        }
    }

    fn sample_profile(tau_entries: Vec<(SlotId, f32)>) -> GuardProfile {
        let mut tau = BTreeMap::new();
        let mut required_slots = Vec::new();
        for (slot, value) in tau_entries {
            tau.insert(slot, value);
            required_slots.push(slot);
        }
        GuardProfile {
            guard_id: guard_id(),
            panel_version: 42,
            domain: "synthetic".to_string(),
            tau,
            required_slots,
            policy: GuardPolicy::AllRequired,
            calibration: None,
            novelty_action: NoveltyAction::Quarantine,
        }
    }

    fn slot_vectors(entries: &[(SlotId, Vec<f32>)]) -> BTreeMap<SlotId, Vec<f32>> {
        entries.iter().cloned().collect()
    }

    fn cos_vector(cos: f32) -> Vec<f32> {
        vec![cos, (1.0 - cos * cos).sqrt()]
    }

    fn assert_close(actual: f32, expected: f32) {
        assert!(
            (actual - expected).abs() <= 1.0e-5,
            "actual={actual} expected={expected}"
        );
    }

    fn manual_cos(a: &[f32; 2], b: &[f32; 2]) -> f32 {
        (a[0] * b[0] + a[1] * b[1]) / (norm(a) * norm(b))
    }

    fn norm(values: &[f32; 2]) -> f32 {
        (values[0] * values[0] + values[1] * values[1]).sqrt()
    }

    fn write_json<T: serde::Serialize>(root: &str, name: &str, value: &T) {
        let path = std::path::Path::new(root).join(name);
        let file = std::fs::File::create(path).expect("create fsv json");
        serde_json::to_writer_pretty(file, value).expect("write fsv json");
    }

    fn guard_id() -> GuardId {
        GUARD_UUID.parse().expect("guard id")
    }

    const fn slot(value: u16) -> SlotId {
        SlotId::new(value)
    }
}
