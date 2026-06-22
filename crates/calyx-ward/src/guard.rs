//! Per-slot Ward guard math.

use std::collections::BTreeMap;

use calyx_core::{SlotId, dense_cosine};

use crate::error::WardError;
use crate::profile::{GuardPolicy, GuardProfile};
use crate::verdict::{GuardVerdict, SlotVerdict};

pub const DEFAULT_TAU: f32 = 0.7;

pub type ProducedSlots = BTreeMap<SlotId, Vec<f32>>;
pub type MatchedSlots = BTreeMap<SlotId, Vec<f32>>;

// INVARIANT A3: required slots are scored independently; no aggregate vector gate.
/// Evaluates every required slot independently under the profile policy.
///
/// Missing required slots fail closed with `WardError::MissingSlot`. Slots with
/// invalid vectors produce a failed slot verdict instead of panicking, preserving
/// the full decomposition for callers and FSV readback.
pub fn guard(
    profile: &GuardProfile,
    produced: &ProducedSlots,
    matched: &MatchedSlots,
    high_stakes: bool,
) -> Result<GuardVerdict, WardError> {
    let required = required_slots(profile);
    validate_non_inert_required(profile, &required)?;
    if high_stakes {
        validate_high_stakes_profile(profile, &required)?;
    }

    let mut per_slot = Vec::new();
    for slot in required {
        let produced_vec = produced.get(&slot).ok_or(WardError::MissingSlot { slot })?;
        let matched_vec = matched.get(&slot).ok_or(WardError::MissingSlot { slot })?;
        let tau = profile.tau_for(&slot).unwrap_or(DEFAULT_TAU);
        let (cos, pass) = match dense_cosine(produced_vec, matched_vec) {
            Some(cos) => (cos, cos >= tau),
            None => (0.0, false),
        };
        per_slot.push(SlotVerdict {
            slot,
            cos,
            tau,
            pass,
        });
    }

    let pass_count = per_slot.iter().filter(|slot| slot.pass).count();
    let overall_pass = match profile.policy {
        GuardPolicy::AllRequired => pass_count == per_slot.len(),
        GuardPolicy::KofN { k } => pass_count >= k,
    };
    let action = (!overall_pass).then(|| profile.novelty_action.clone());
    Ok(GuardVerdict {
        guard_id: profile.guard_id,
        overall_pass,
        provisional: !profile.is_calibrated(),
        per_slot,
        action,
    })
}

/// Rejects guard profiles that cannot require evidence from any slot.
pub fn validate_non_inert_profile(profile: &GuardProfile) -> Result<(), WardError> {
    let required = required_slots(profile);
    validate_non_inert_required(profile, &required)
}

fn validate_non_inert_required(
    profile: &GuardProfile,
    required: &[SlotId],
) -> Result<(), WardError> {
    if required.is_empty() {
        return Err(WardError::InertProfile {
            guard_id: profile.guard_id,
            reason: "empty_required_slots",
        });
    }
    if let GuardPolicy::KofN { k } = profile.policy {
        if k == 0 {
            return Err(WardError::InertProfile {
                guard_id: profile.guard_id,
                reason: "kofn_zero",
            });
        }
        if k > required.len() {
            return Err(WardError::PolicyViolation {
                k,
                n_required: required.len(),
            });
        }
    }
    Ok(())
}

fn validate_high_stakes_profile(
    profile: &GuardProfile,
    required: &[SlotId],
) -> Result<(), WardError> {
    let calibration = profile.calibration.as_ref().ok_or(WardError::Provisional {
        guard_id: profile.guard_id,
    })?;
    for slot in required {
        if profile.tau_for(slot).is_none() || !calibration.per_slot.contains_key(slot) {
            return Err(WardError::MissingSlotCalibration {
                guard_id: profile.guard_id,
                slot: *slot,
            });
        }
    }
    Ok(())
}

/// Runs `guard()` for non-critical embeddings that may use provisional profiles.
pub fn guard_non_high_stakes(
    profile: &GuardProfile,
    produced: &ProducedSlots,
    matched: &MatchedSlots,
) -> Result<GuardVerdict, WardError> {
    guard(profile, produced, matched, false)
}

/// Runs `guard()` and returns `WardError::Ood` when the verdict does not pass.
pub fn guard_result(
    profile: &GuardProfile,
    produced: &ProducedSlots,
    matched: &MatchedSlots,
) -> Result<GuardVerdict, WardError> {
    guard_result_with_stakes(profile, produced, matched, false)
}

/// Runs `guard()` with the caller's stake level and wraps non-pass as OOD.
pub fn guard_result_with_stakes(
    profile: &GuardProfile,
    produced: &ProducedSlots,
    matched: &MatchedSlots,
    high_stakes: bool,
) -> Result<GuardVerdict, WardError> {
    let verdict = guard(profile, produced, matched, high_stakes)?;
    if verdict.overall_pass {
        Ok(verdict)
    } else {
        Err(WardError::Ood {
            guard_id: verdict.guard_id,
            failing: verdict
                .per_slot
                .iter()
                .filter(|slot| !slot.pass)
                .cloned()
                .collect(),
        })
    }
}

fn required_slots(profile: &GuardProfile) -> Vec<SlotId> {
    let mut slots = profile.required_slots.clone();
    slots.sort_unstable();
    slots.dedup();
    slots
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

    fn guard(
        profile: &GuardProfile,
        produced: &ProducedSlots,
        matched: &MatchedSlots,
    ) -> Result<GuardVerdict, WardError> {
        super::guard(profile, produced, matched, false)
    }

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
    fn empty_required_slots_fails_inert_profile() {
        let profile = sample_profile(vec![]);
        let produced = ProducedSlots::new();
        let matched = MatchedSlots::new();

        let error = guard(&profile, &produced, &matched).expect_err("inert profile");

        assert_eq!(
            error,
            WardError::InertProfile {
                guard_id: guard_id(),
                reason: "empty_required_slots",
            }
        );
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
    fn invalid_vector_fails_even_when_tau_is_zero() {
        let profile = sample_profile(vec![(slot(1), 0.0)]);
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
    #[ignore = "manual FSV fixture; set CALYX_WARD_GUARD_FSV_DIR"]
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
        .expect_err("empty required slots");
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
        write_json(&root, "edge-empty-required-error.json", &error_json(&empty));
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

    fn error_json(error: &WardError) -> serde_json::Value {
        json!({
            "code": error.code(),
            "message": error.to_string(),
        })
    }

    fn guard_id() -> GuardId {
        GUARD_UUID.parse().expect("guard id")
    }

    const fn slot(value: u16) -> SlotId {
        SlotId::new(value)
    }
}
