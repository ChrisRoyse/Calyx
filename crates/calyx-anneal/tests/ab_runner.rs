use std::fs;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use calyx_anneal::{
    ABLedgerEvent, ABLedgerWriter, ABResult, ABRunner, ABVerdict, AnnealLedgerAction, BanditPolicy,
    CALYX_ANNEAL_TRIAL_ALREADY_ACTIVE, ConfigBandit, DType, NoopABBudget, ShapeKey,
    TripwireRegistry,
};
use calyx_core::{FixedClock, Result};
use proptest::prelude::*;

const TEST_TS: u64 = 1_785_500_416;

#[test]
fn candidate_promotes_after_min_samples_when_faster_without_quality_regression() {
    let mut bandit = make_bandit();
    let mut runner = make_runner(NoopABBudget::default());
    let key = shape_key();
    runner
        .start_trial_with_config(key.clone(), 1, 0, 10, None)
        .unwrap();

    let mut verdict = None;
    for _ in 0..10 {
        verdict = runner
            .record_query(
                &key,
                result(0, 100, 0.95, 1.2),
                result(1, 70, 0.95, 1.2),
                &mut bandit,
            )
            .unwrap();
    }

    let ABVerdict::Promoted(record) = verdict.expect("verdict") else {
        panic!("expected promotion");
    };
    assert_eq!(record.samples, 10);
    assert!(record.latency_after_ns < record.latency_before_ns);
    assert_eq!(bandit.incumbent_idx, 1);
    assert_eq!(runner.writer.events.len(), 1);
    assert_eq!(
        runner.writer.events[0].action,
        AnnealLedgerAction::AutotunePromote
    );
}

#[test]
fn latency_win_keeps_incumbent_when_recall_regresses() {
    let mut bandit = make_bandit();
    let mut runner = make_runner(NoopABBudget::default());
    let key = shape_key();
    runner
        .start_trial_with_config(key.clone(), 1, 0, 3, None)
        .unwrap();

    let mut verdict = None;
    for _ in 0..3 {
        verdict = runner
            .record_query(
                &key,
                result(0, 100, 0.95, 1.0),
                result(1, 70, 0.89, 1.0),
                &mut bandit,
            )
            .unwrap();
    }

    let ABVerdict::Kept(record) = verdict.expect("verdict") else {
        panic!("expected incumbent keep");
    };
    assert_eq!(record.reason, "recall_tripwire");
    assert_eq!(bandit.incumbent_idx, 0);
    assert_eq!(
        runner.writer.events[0].action,
        AnnealLedgerAction::AutotuneAB
    );
}

#[test]
fn verdict_is_idempotent_after_trial_completes() {
    let mut bandit = make_bandit();
    let mut runner = make_runner(NoopABBudget::default());
    let key = shape_key();
    runner
        .start_trial_with_config(key.clone(), 1, 0, 1, None)
        .unwrap();

    let first = runner
        .record_query(
            &key,
            result(0, 100, 0.95, 1.0),
            result(1, 70, 0.95, 1.0),
            &mut bandit,
        )
        .unwrap()
        .expect("first verdict");
    let second = runner
        .record_query(
            &key,
            result(0, 90, 0.95, 1.0),
            result(1, 60, 0.95, 1.0),
            &mut bandit,
        )
        .unwrap()
        .expect("second verdict");

    assert_eq!(first, second);
    assert_eq!(runner.writer.events.len(), 1);

    runner
        .start_trial_with_config(key.clone(), 0, 1, 1, None)
        .unwrap();
    let restarted = runner.active_trials.get(&key).expect("restarted trial");
    assert_eq!(restarted.candidate_idx, 0);
    assert_eq!(restarted.incumbent_idx, 1);
    assert_eq!(restarted.query_pairs(), 0);
    assert!(restarted.verdict.is_none());
}

#[test]
fn duplicate_trial_fails_closed_and_budget_exhaustion_abandons() {
    let mut runner = make_runner(NoopABBudget::default());
    let key = shape_key();
    runner.start_trial(key.clone(), 1, 0).unwrap();
    let error = runner.start_trial(key.clone(), 1, 0).unwrap_err();
    assert_eq!(error.code, CALYX_ANNEAL_TRIAL_ALREADY_ACTIVE);

    let mut bandit = make_bandit();
    let mut exhausted = make_runner(NoopABBudget { ticks: 0 });
    exhausted
        .start_trial_with_config(key.clone(), 1, 0, 1, None)
        .unwrap();
    let verdict = exhausted
        .record_query(
            &key,
            result(0, 100, 0.95, 1.0),
            result(1, 70, 0.95, 1.0),
            &mut bandit,
        )
        .unwrap()
        .expect("abandoned verdict");
    let ABVerdict::Abandoned(record) = verdict else {
        panic!("expected abandoned verdict");
    };
    assert_eq!(record.samples, 0);
    assert_eq!(
        exhausted.writer.events[0].action,
        AnnealLedgerAction::AutotuneAbandoned
    );
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]

    #[test]
    fn promotion_iff_candidate_p99_faster_and_no_tripwire_or_quality_regression(
        incumbent_latency in 50u64..300,
        candidate_latency in 1u64..300,
        incumbent_recall_milli in 900u32..=1000,
        candidate_recall_milli in 850u32..=1000,
        candidate_bits_centi in 50u32..=150,
    ) {
        let mut bandit = make_bandit();
        let mut runner = make_runner(NoopABBudget::default());
        let key = shape_key();
        runner.start_trial_with_config(key.clone(), 1, 0, 1, None).unwrap();
        let incumbent_recall = incumbent_recall_milli as f64 / 1000.0;
        let candidate_recall = candidate_recall_milli as f64 / 1000.0;
        let candidate_bits = candidate_bits_centi as f64 / 100.0;
        let verdict = runner
            .record_query(
                &key,
                result(0, incumbent_latency, incumbent_recall, 1.0),
                result(1, candidate_latency, candidate_recall, candidate_bits),
                &mut bandit,
            )
            .unwrap()
            .expect("verdict");

        let expected = candidate_latency < incumbent_latency
            && candidate_latency <= 200
            && candidate_recall >= 0.90
            && candidate_recall + f64::EPSILON >= incumbent_recall
            && candidate_bits + f64::EPSILON >= 1.0;
        prop_assert_eq!(matches!(verdict, ABVerdict::Promoted(_)), expected);
    }
}

#[derive(Default)]
struct RecordingWriter {
    events: Vec<ABLedgerEvent>,
}

impl ABLedgerWriter for RecordingWriter {
    fn write_ab_event(&mut self, event: &ABLedgerEvent) -> Result<()> {
        self.events.push(event.clone());
        Ok(())
    }
}

fn make_runner(budget: NoopABBudget) -> ABRunner<RecordingWriter, NoopABBudget> {
    ABRunner::new(
        tripwires(),
        RecordingWriter::default(),
        budget,
        Arc::new(FixedClock::new(TEST_TS)),
    )
}

fn make_bandit() -> ConfigBandit {
    let mut bandit =
        ConfigBandit::new(BanditPolicy::EpsilonGreedy { epsilon: 0.0 }, 416).with_hysteresis(1);
    bandit.add_arm(b"incumbent".to_vec());
    bandit.add_arm(b"candidate".to_vec());
    bandit
}

fn result(arm_idx: usize, latency_ns: u64, recall_k: f64, bits_per_anchor: f64) -> ABResult {
    ABResult {
        arm_idx,
        latency_ns,
        recall_k,
        bits_per_anchor,
        ts: TEST_TS,
    }
}

fn shape_key() -> ShapeKey {
    ShapeKey::new("ab-runner-test", &[128, 64], DType::Fp32, "cpu0")
}

fn tripwires() -> TripwireRegistry {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    let dir = std::env::temp_dir().join(format!(
        "calyx-ab-runner-test-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&dir);
    let registry = TripwireRegistry::load_from_vault(&dir).unwrap();
    let _ = fs::remove_dir_all(&dir);
    registry
}
