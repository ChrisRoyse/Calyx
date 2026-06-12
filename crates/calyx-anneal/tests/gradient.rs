use std::{
    fs,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use calyx_anneal::{
    CandidateAction, ComponentKind, GradientCandidate, IntelligenceGradient, JTerms, JValue,
    JWeights, PriorityReadback, ScopeId, TuneScopeKind, estimate_dj, gradient_state_path,
    read_gradient_snapshot_from_vault, write_gradient_snapshot,
};
use calyx_core::{FixedClock, LensId, SlotId};
use proptest::prelude::*;

#[test]
fn next_best_action_returns_highest_dj_per_cost() {
    let mut gradient = gradient();
    gradient.refresh(vec![
        candidate(action_label("slow", 0.5), 1),
        candidate(action_label("top", 2.0), 1),
        candidate(action_label("mid", 1.0), 1),
    ]);

    assert!(matches!(
        gradient.next_best_action(),
        Some(CandidateAction::LabelAnchor { anchor, .. }) if anchor.as_str() == "top"
    ));
}

#[test]
fn propose_lens_estimate_uses_known_information_delta() {
    let action =
        CandidateAction::propose_lens_from_info(anchor("quality"), 0.3, 0.8).expect("known delta");

    assert!(matches!(
        &action,
        CandidateAction::ProposeLens { estimated_dj, .. } if (*estimated_dj - 0.5).abs() < 1e-12
    ));
    assert!((estimate_dj(&action, JWeights::default()).unwrap() - 0.5).abs() < 1e-12);
}

#[test]
fn empty_queue_returns_none_and_zero_ties_keep_insertion_order() {
    let mut gradient = gradient();

    assert!(gradient.next_best_action().is_none());

    gradient.refresh(vec![
        candidate(action_label("first", 0.0), 4),
        candidate(action_label("second", 0.0), 4),
    ]);

    assert!(matches!(
        gradient.next_best_action(),
        Some(CandidateAction::LabelAnchor { anchor, .. }) if anchor.as_str() == "first"
    ));
}

#[test]
fn zero_cost_candidate_has_infinite_priority() {
    let mut gradient = gradient();
    gradient.refresh(vec![candidate(action_label("free", 0.25), 0)]);
    let top = gradient.top_entries(1).pop().expect("top entry");

    assert!(top.dj_per_cost.is_infinite());
    assert!(matches!(
        top.to_readback().dj_per_cost,
        PriorityReadback::Infinite
    ));
}

#[test]
fn invalid_nan_estimate_is_excluded_with_warning() {
    let mut gradient = gradient();
    let report = gradient.refresh(vec![
        candidate(action_label("bad", f64::NAN), 1),
        candidate(action_label("good", 0.3), 1),
    ]);

    assert_eq!(report.accepted, 1);
    assert_eq!(report.rejected.len(), 1);
    assert_eq!(
        report.rejected[0].code,
        "CALYX_ANNEAL_GRADIENT_INVALID_METRIC"
    );
    assert!(matches!(
        gradient.next_best_action(),
        Some(CandidateAction::LabelAnchor { anchor, .. }) if anchor.as_str() == "good"
    ));
}

#[test]
fn over_budget_candidate_is_filtered_out() {
    let mut gradient = gradient().with_budget_units(5);
    let report = gradient.refresh(vec![
        candidate(action_label("too-expensive", 10.0), 6),
        candidate(action_label("affordable", 1.0), 5),
    ]);

    assert_eq!(report.accepted, 1);
    assert_eq!(report.rejected[0].code, "CALYX_ANNEAL_GRADIENT_OVER_BUDGET");
    assert!(matches!(
        gradient.next_best_action(),
        Some(CandidateAction::LabelAnchor { anchor, .. }) if anchor.as_str() == "affordable"
    ));
}

#[test]
fn gradient_snapshot_roundtrips_to_vault_bytes() {
    let root = std::env::temp_dir().join(format!(
        "calyx-gradient-test-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos()
    ));
    fs::create_dir_all(&root).expect("tempdir");
    let mut gradient = gradient();
    gradient.refresh(vec![
        candidate(action_recompute_kernel("domain-a", 0.6), 2),
        candidate(action_retune_math(0.9), 3),
    ]);
    let snapshot = gradient.snapshot(5);
    let path = write_gradient_snapshot(&root, &snapshot).expect("write snapshot");

    assert_eq!(path, gradient_state_path(&root));
    let readback = read_gradient_snapshot_from_vault(&root)
        .expect("read snapshot")
        .expect("snapshot exists");
    assert_eq!(readback.generated_at, 1_785_500_425);
    assert_eq!(readback.gradient.len(), 2);
    assert!(readback.next_best_action.is_some());
    assert!(root.starts_with(std::env::temp_dir()));
    fs::remove_dir_all(&root).expect("remove tempdir");
}

proptest! {
    #[test]
    fn next_best_action_matches_highest_priority(values in proptest::collection::vec(0.0f64..100.0, 1..32)) {
        let mut gradient = gradient();
        let candidates: Vec<_> = values
            .iter()
            .enumerate()
            .map(|(index, value)| candidate(action_label(format!("a{index}"), *value), 1))
            .collect();
        let expected_index = values
            .iter()
            .enumerate()
            .max_by(|left, right| left.1.total_cmp(right.1))
            .map(|(index, _)| index)
            .expect("non-empty");

        gradient.refresh(candidates);

        let expected_anchor = format!("a{}", expected_index);
        if let Some(CandidateAction::LabelAnchor { anchor, .. }) = gradient.next_best_action() {
            prop_assert_eq!(anchor.as_str(), expected_anchor.as_str());
        } else {
            prop_assert!(false);
        }
    }
}

fn gradient() -> IntelligenceGradient {
    IntelligenceGradient::new(j(), Arc::new(FixedClock::new(1_785_500_425)))
}

fn candidate(action: CandidateAction, cost_budget_units: u64) -> GradientCandidate {
    GradientCandidate {
        action,
        cost_budget_units,
    }
}

fn action_label(anchor_value: impl Into<String>, estimated_dj: f64) -> CandidateAction {
    CandidateAction::LabelAnchor {
        anchor: anchor(anchor_value),
        estimated_dj,
    }
}

fn action_recompute_kernel(scope: impl Into<String>, estimated_dj: f64) -> CandidateAction {
    CandidateAction::RecomputeKernel {
        scope: ScopeId::new(scope),
        estimated_dj,
    }
}

fn action_retune_math(estimated_dj: f64) -> CandidateAction {
    CandidateAction::RetuneMath {
        scope: TuneScopeKind::AnnIndex,
        estimated_dj,
    }
}

fn anchor(value: impl Into<String>) -> calyx_anneal::AnchorId {
    calyx_anneal::AnchorId::new(value).unwrap()
}

fn lens(byte: u8) -> LensId {
    LensId::from_bytes([byte; 16])
}

fn j() -> JValue {
    JValue {
        j: 7.0,
        terms: JTerms {
            w1_info: 2.0,
            w2_n_eff: 1.0,
            w3_sufficiency: 1.0,
            w4_kernel_recall: 1.0,
            w5_oracle_accuracy: 1.0,
            w6_mistake_rate: 0.2,
            w7_compression: 1.0,
            w8_coverage: 0.2,
            p_redundant: 0.0,
            p_ungrounded: 0.0,
            p_goodhart: 0.0,
        },
        dpi_ceiling: 10.0,
        dpi_headroom: 8.0,
        provisional_excluded: 0,
        weights: JWeights::default(),
    }
}

#[test]
fn all_action_weights_map_to_the_expected_j_terms() {
    let weights = JWeights {
        w1: 2.0,
        w2: 3.0,
        w3: 1.0,
        w4: 4.0,
        w5: 5.0,
        w6: 1.0,
        w7: 7.0,
        w8: 1.0,
    };
    let l1 = lens(1);
    let l2 = lens(2);

    assert_eq!(
        estimate_dj(&action_label("anchor", 0.5), weights).unwrap(),
        1.0
    );
    assert_eq!(
        estimate_dj(
            &CandidateAction::PruneRedundantLens {
                lens_id: l1,
                estimated_dj: 0.5
            },
            weights
        )
        .unwrap(),
        1.5
    );
    assert_eq!(
        estimate_dj(&action_recompute_kernel("scope", 0.5), weights).unwrap(),
        2.0
    );
    assert_eq!(
        estimate_dj(
            &CandidateAction::RecalibrateHeal {
                component: ComponentKind::ann_index(SlotId::new(1)),
                estimated_dj: 0.5
            },
            weights
        )
        .unwrap(),
        2.5
    );
    assert_eq!(
        estimate_dj(
            &CandidateAction::MaterializeCrossTerm {
                pair: (l1, l2),
                estimated_dj: 0.5
            },
            weights
        )
        .unwrap(),
        1.0
    );
    assert_eq!(estimate_dj(&action_retune_math(0.5), weights).unwrap(), 3.5);
}
