use calyx_anneal::{
    CALYX_REGISTRY_HOT_ADD_FAIL, ChangeId, GateOutcome, ProposalTerminalState, ProposeLens,
    ProposeLensRequest, ShadowRevertReason,
};
use calyx_core::FixedClock;

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
