use calyx_anneal::{
    CALYX_REGISTRY_HOT_ADD_FAIL, CandidateLens, ChangeId, GateOutcome, ProposalTerminalState,
    ProposeLens, ProposeLensRequest, RegistryHotAdder, RejectReason, ShadowRevertReason,
};
use calyx_assay::PanelResourceBudget;
use calyx_core::{FixedClock, Modality};
use calyx_registry::{CapabilitySignalKind, CostMetrics, Registry};

#[path = "support/propose_lens.rs"]
mod support;
use support::*;

#[test]
fn admitted_candidate_hot_adds_and_improves_sufficiency() {
    let clock = FixedClock::new(TEST_TS);
    let anchor = anchor();
    let mut controller = controller();
    let mut substrate = TestSubstrate::promote(ChangeId(421_001));
    let assay = FixtureAssay::new([0.20, 0.80], 1.00);
    let profiler = StaticProfiler::new(0.12);
    let nmi = StaticNmi::new(0.45);
    let mut hot_add = TestHotAdder::succeed();
    let corpus = corpus();

    let outcome = ProposeLens::new(&clock)
        .propose_lens(ProposeLensRequest {
            anchor: &anchor,
            controller: &mut controller,
            substrate: &mut substrate,
            assay: &assay,
            hot_add: &mut hot_add,
            profiler: &profiler,
            nmi: &nmi,
            corpus: &corpus,
        })
        .unwrap();

    assert!(outcome.admitted);
    assert_eq!(outcome.terminal_state, ProposalTerminalState::Admitted);
    assert_eq!(outcome.sufficiency_before, 0.20);
    assert_eq!(outcome.sufficiency_after, Some(0.80));
    assert_eq!(outcome.change_id, Some(ChangeId(421_001)));
    assert_eq!(controller.panel().slots.len(), 2);
    assert_eq!(hot_add.apply_calls, 1);
    assert!(substrate.rolled_back.is_empty());
}

#[test]
fn rejected_gate_skips_substrate_and_hot_add() {
    let clock = FixedClock::new(TEST_TS);
    let mut controller = controller();
    let mut substrate = TestSubstrate::promote(ChangeId(421_002));
    let assay = FixtureAssay::new([0.20], 1.00);
    let profiler = StaticProfiler::new(0.01);
    let nmi = StaticNmi::new(0.10);
    let mut hot_add = TestHotAdder::succeed();
    let anchor = anchor();
    let corpus = corpus();

    let outcome = ProposeLens::new(&clock)
        .propose_lens(ProposeLensRequest {
            anchor: &anchor,
            controller: &mut controller,
            substrate: &mut substrate,
            assay: &assay,
            hot_add: &mut hot_add,
            profiler: &profiler,
            nmi: &nmi,
            corpus: &corpus,
        })
        .unwrap();

    assert_eq!(outcome.terminal_state, ProposalTerminalState::GateRejected);
    assert!(matches!(
        outcome.gate_outcome,
        Some(GateOutcome::Rejected { .. })
    ));
    assert_eq!(controller.panel().slots.len(), 1);
    assert_eq!(substrate.proposed, 0);
    assert_eq!(hot_add.apply_calls, 0);
}

#[test]
fn placeholder_signal_rejection_skips_substrate_and_hot_add() {
    let clock = FixedClock::new(TEST_TS);
    let mut controller = controller();
    let mut substrate = TestSubstrate::promote(ChangeId(421_008));
    let assay = FixtureAssay::new([0.20], 1.00);
    let profiler = StaticProfiler::new(0.90).with_signal_kind(CapabilitySignalKind::Placeholder);
    let nmi = StaticNmi::new(0.10);
    let mut hot_add = TestHotAdder::succeed();
    let anchor = anchor();
    let corpus = corpus();

    let outcome = ProposeLens::new(&clock)
        .propose_lens(ProposeLensRequest {
            anchor: &anchor,
            controller: &mut controller,
            substrate: &mut substrate,
            assay: &assay,
            hot_add: &mut hot_add,
            profiler: &profiler,
            nmi: &nmi,
            corpus: &corpus,
        })
        .unwrap();

    assert_eq!(outcome.terminal_state, ProposalTerminalState::GateRejected);
    assert!(matches!(
        outcome.gate_outcome,
        Some(GateOutcome::Rejected {
            reason: RejectReason::NonLearnedSignal { .. }
        })
    ));
    assert_eq!(controller.panel().slots.len(), 1);
    assert_eq!(substrate.proposed, 0);
    assert_eq!(hot_add.apply_calls, 0);
}

#[test]
fn resource_budget_rejection_skips_substrate_and_hot_add() {
    let clock = FixedClock::new(TEST_TS);
    let mut controller = controller();
    let mut substrate = TestSubstrate::promote(ChangeId(421_007));
    let assay = FixtureAssay::new([0.20], 1.00);
    let profiler = StaticProfiler::new(0.12).with_cost(CostMetrics {
        total_ms: 1.0,
        ms_per_input: 1.0,
        vram_bytes: 512 * 1024 * 1024,
        ram_bytes: 0,
        batch_ceiling: 1,
    });
    let nmi = StaticNmi::new(0.10);
    let mut hot_add = TestHotAdder::succeed();
    let anchor = anchor();
    let corpus = corpus();
    let budget = PanelResourceBudget {
        max_vram_mb: 128.0,
        max_ram_mb: 1024.0,
        max_ms_per_input: 5.0,
    };

    let outcome = ProposeLens::new(&clock)
        .with_resource_budget(budget)
        .propose_lens(ProposeLensRequest {
            anchor: &anchor,
            controller: &mut controller,
            substrate: &mut substrate,
            assay: &assay,
            hot_add: &mut hot_add,
            profiler: &profiler,
            nmi: &nmi,
            corpus: &corpus,
        })
        .unwrap();

    assert_eq!(outcome.terminal_state, ProposalTerminalState::GateRejected);
    assert!(matches!(
        outcome.gate_outcome,
        Some(GateOutcome::Rejected {
            reason: RejectReason::ResourceBudgetExceeded { .. }
        })
    ));
    assert_eq!(controller.panel().slots.len(), 1);
    assert_eq!(substrate.proposed, 0);
    assert_eq!(hot_add.apply_calls, 0);
}

#[test]
fn no_deficit_returns_before_synthesis() {
    let clock = FixedClock::new(TEST_TS);
    let mut controller = controller();
    let mut substrate = TestSubstrate::promote(ChangeId(421_003));
    let assay = FixtureAssay::new([0.95], 1.00);
    let profiler = StaticProfiler::new(0.12);
    let nmi = StaticNmi::new(0.10);
    let mut hot_add = TestHotAdder::succeed();
    let anchor = anchor();
    let corpus = Vec::new();

    let outcome = ProposeLens::new(&clock)
        .propose_lens(ProposeLensRequest {
            anchor: &anchor,
            controller: &mut controller,
            substrate: &mut substrate,
            assay: &assay,
            hot_add: &mut hot_add,
            profiler: &profiler,
            nmi: &nmi,
            corpus: &corpus,
        })
        .unwrap();

    assert_eq!(outcome.terminal_state, ProposalTerminalState::NoDeficit);
    assert_eq!(outcome.candidate, None);
    assert_eq!(outcome.gate_outcome, None);
    assert_eq!(controller.panel().slots.len(), 1);
    assert_eq!(substrate.proposed, 0);
    assert_eq!(hot_add.apply_calls, 0);
}

#[test]
fn substrate_revert_leaves_panel_unchanged() {
    let clock = FixedClock::new(TEST_TS);
    let mut controller = controller();
    let mut substrate =
        TestSubstrate::revert(ChangeId(421_004), ShadowRevertReason::BudgetExhausted);
    let assay = FixtureAssay::new([0.20], 1.00);
    let profiler = StaticProfiler::new(0.12);
    let nmi = StaticNmi::new(0.10);
    let mut hot_add = TestHotAdder::succeed();
    let anchor = anchor();
    let corpus = corpus();

    let outcome = ProposeLens::new(&clock)
        .propose_lens(ProposeLensRequest {
            anchor: &anchor,
            controller: &mut controller,
            substrate: &mut substrate,
            assay: &assay,
            hot_add: &mut hot_add,
            profiler: &profiler,
            nmi: &nmi,
            corpus: &corpus,
        })
        .unwrap();

    assert_eq!(
        outcome.terminal_state,
        ProposalTerminalState::SubstrateReverted {
            reason: ShadowRevertReason::BudgetExhausted
        }
    );
    assert_eq!(outcome.change_id, Some(ChangeId(421_004)));
    assert_eq!(controller.panel().slots.len(), 1);
    assert_eq!(hot_add.apply_calls, 0);
}

#[test]
fn no_sufficiency_gain_rolls_back_panel() {
    let clock = FixedClock::new(TEST_TS);
    let mut controller = controller();
    let mut substrate = TestSubstrate::promote(ChangeId(421_005));
    let assay = FixtureAssay::new([0.20, 0.20], 1.00);
    let profiler = StaticProfiler::new(0.12);
    let nmi = StaticNmi::new(0.10);
    let mut hot_add = TestHotAdder::succeed();
    let anchor = anchor();
    let corpus = corpus();

    let outcome = ProposeLens::new(&clock)
        .propose_lens(ProposeLensRequest {
            anchor: &anchor,
            controller: &mut controller,
            substrate: &mut substrate,
            assay: &assay,
            hot_add: &mut hot_add,
            profiler: &profiler,
            nmi: &nmi,
            corpus: &corpus,
        })
        .unwrap();

    assert_eq!(
        outcome.terminal_state,
        ProposalTerminalState::NoSufficiencyGain
    );
    assert_eq!(outcome.sufficiency_after, Some(0.20));
    assert_eq!(controller.panel().slots.len(), 1);
    assert_eq!(substrate.rolled_back, vec![ChangeId(421_005)]);
}

#[test]
fn hot_add_failure_restores_panel_and_rolls_back() {
    let clock = FixedClock::new(TEST_TS);
    let mut controller = controller();
    let mut substrate = TestSubstrate::promote(ChangeId(421_006));
    let assay = FixtureAssay::new([0.20], 1.00);
    let profiler = StaticProfiler::new(0.12);
    let nmi = StaticNmi::new(0.10);
    let mut hot_add = TestHotAdder::fail_after_mutate();
    let anchor = anchor();
    let corpus = corpus();

    let outcome = ProposeLens::new(&clock)
        .propose_lens(ProposeLensRequest {
            anchor: &anchor,
            controller: &mut controller,
            substrate: &mut substrate,
            assay: &assay,
            hot_add: &mut hot_add,
            profiler: &profiler,
            nmi: &nmi,
            corpus: &corpus,
        })
        .unwrap();

    assert_eq!(
        outcome.terminal_state,
        ProposalTerminalState::HotAddFailed {
            code: CALYX_REGISTRY_HOT_ADD_FAIL.to_string()
        }
    );
    assert_eq!(controller.panel().slots.len(), 1);
    assert_eq!(substrate.rolled_back, vec![ChangeId(421_006)]);
}

#[test]
fn commissioned_conversion_target_hot_adds_factory_artifact_and_protein_slot() {
    let root = std::env::temp_dir().join(format!(
        "calyx-issue791-registry-hot-add-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("create issue791 artifact root");
    let clock = FixedClock::new(TEST_TS);
    let mut controller = controller();
    let mut substrate = TestSubstrate::promote(ChangeId(791_001));
    let assay =
        FixtureAssay::new([0.20, 0.85], 1.40).with_expected_modalities(vec![Modality::Protein]);
    let profiler = StaticProfiler::new(0.14);
    let nmi = StaticNmi::new(0.30);
    let mut registry = Registry::new();
    let mut hot_add = RegistryHotAdder::with_artifact_dir(&mut registry, &root);
    let anchor = anchor();
    let corpus = corpus();

    let outcome = ProposeLens::new(&clock)
        .propose_lens(ProposeLensRequest {
            anchor: &anchor,
            controller: &mut controller,
            substrate: &mut substrate,
            assay: &assay,
            hot_add: &mut hot_add,
            profiler: &profiler,
            nmi: &nmi,
            corpus: &corpus,
        })
        .unwrap();

    assert_eq!(outcome.terminal_state, ProposalTerminalState::Admitted);
    let CandidateLens::Commission { spec } = outcome.candidate.as_ref().unwrap() else {
        panic!("expected commissioned conversion target");
    };
    assert_eq!(spec.target_modality, Modality::Protein);
    assert_eq!(spec.axis, "protein_sequence");
    assert_eq!(spec.suggested_targets[0].hf_id, "facebook/esm2_t6_8M_UR50D");
    let slot = controller.panel().slots.last().expect("hot-added slot");
    assert_eq!(slot.modality, Modality::Protein);
    assert_eq!(slot.axis.as_deref(), Some("protein_sequence"));
    assert_eq!(controller.panel().slots.len(), 2);
    assert_eq!(substrate.proposed, 1);
    assert!(substrate.rolled_back.is_empty());
    assert!(commissioned_artifact_exists(&root));

    let _ = std::fs::remove_dir_all(root);
}

fn commissioned_artifact_exists(root: &std::path::Path) -> bool {
    let Ok(entries) = std::fs::read_dir(root) else {
        return false;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() && commissioned_artifact_exists(&path) {
            return true;
        }
        if path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.ends_with(".commissioned.json"))
        {
            return true;
        }
    }
    false
}
