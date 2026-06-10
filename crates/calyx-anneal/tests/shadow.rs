use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use calyx_anneal::{
    ActionMetricSnapshot, AnnealAction, BudgetHandle, HeldOutReplay, MetricComparison, MetricSide,
    MetricSnapshot, ReplayAnchor, ReplayQuery, ShadowExecutor, ShadowRevertReason, ShadowVerdict,
    TripwireMetric, TripwireRegistry,
};
use calyx_core::{CxId, FixedClock};
use proptest::prelude::*;

const TEST_TS: u64 = 1_785_500_395;

struct TestRoot(PathBuf);

impl TestRoot {
    fn new(label: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "calyx-shadow-{label}-{}-{}",
            std::process::id(),
            NEXT_ROOT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("create temp root");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

static NEXT_ROOT: AtomicUsize = AtomicUsize::new(0);

struct FixedAction {
    values: ActionMetricSnapshot,
    calls: AtomicUsize,
}

impl FixedAction {
    fn new(values: impl IntoIterator<Item = (TripwireMetric, f64)>) -> Self {
        Self {
            values: ActionMetricSnapshot::from_values(values),
            calls: AtomicUsize::new(0),
        }
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl AnnealAction for FixedAction {
    fn apply_shadow(&self, _query: &ReplayQuery) -> ActionMetricSnapshot {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.values.clone()
    }
}

#[test]
fn recall_tripwire_cross_reverts_and_keeps_metric_pairs() {
    let root = TestRoot::new("recall");
    let clock = FixedClock::new(TEST_TS);
    let registry = TripwireRegistry::load_from_vault(root.path()).expect("tripwire registry");
    let replay = replay(2);
    let candidate = FixedAction::new(values(0.85, 0.005, 0.02, 120.0, 250.0));
    let incumbent = FixedAction::new(values(1.0, 0.006, 0.03, 130.0, 300.0));
    let mut executor = ShadowExecutor::new(registry, replay, BudgetHandle::new(2), &clock);

    let verdict = executor.run_shadow(&candidate, &incumbent);

    match verdict {
        ShadowVerdict::Revert { reason, metrics } => {
            assert_eq!(
                reason,
                ShadowRevertReason::TripwireCrossed(TripwireMetric::RecallAtK)
            );
            assert_eq!(metrics.evaluated_at, TEST_TS);
            assert_eq!(metrics.query_count, 2);
            assert_eq!(
                comparison(&metrics, TripwireMetric::RecallAtK),
                MetricComparison {
                    metric: TripwireMetric::RecallAtK,
                    candidate_value: 0.85,
                    incumbent_value: 1.0,
                }
            );
        }
        other => panic!("expected recall revert, got {other:?}"),
    }
}

#[test]
fn candidate_dominates_every_metric_promotes() {
    let root = TestRoot::new("promote");
    let clock = FixedClock::new(TEST_TS);
    let registry = TripwireRegistry::load_from_vault(root.path()).expect("tripwire registry");
    let candidate = FixedAction::new(values(0.97, 0.004, 0.015, 110.0, 240.0));
    let incumbent = FixedAction::new(values(0.92, 0.008, 0.03, 150.0, 320.0));
    let mut executor = ShadowExecutor::new(registry, replay(3), BudgetHandle::new(3), &clock);

    let verdict = executor.run_shadow(&candidate, &incumbent);

    match verdict {
        ShadowVerdict::Promote { metrics } => {
            assert_eq!(metrics.evaluated_at, TEST_TS);
            assert_eq!(metrics.query_count, 3);
            assert_eq!(metrics.metrics.len(), 5);
            assert_eq!(
                comparison(&metrics, TripwireMetric::SearchP99).candidate_value,
                110.0
            );
        }
        other => panic!("expected promote, got {other:?}"),
    }
}

#[test]
fn empty_single_equal_and_budget_zero_edges_are_fail_closed() {
    let root = TestRoot::new("edges");
    let clock = FixedClock::new(TEST_TS);
    let registry = TripwireRegistry::load_from_vault(root.path()).expect("registry");
    let candidate = FixedAction::new(values(0.95, 0.005, 0.02, 120.0, 250.0));
    let incumbent = FixedAction::new(values(0.95, 0.005, 0.02, 120.0, 250.0));
    let mut executor = ShadowExecutor::new(registry, replay(0), BudgetHandle::new(1), &clock);

    assert!(matches!(
        executor.run_shadow(&candidate, &incumbent),
        ShadowVerdict::Revert {
            reason: ShadowRevertReason::InsufficientReplay,
            ..
        }
    ));
    assert_eq!(candidate.calls(), 0);
    assert_eq!(incumbent.calls(), 0);

    let root = TestRoot::new("single");
    let registry = TripwireRegistry::load_from_vault(root.path()).expect("registry");
    let mut executor = ShadowExecutor::new(registry, replay(1), BudgetHandle::new(1), &clock);
    assert!(matches!(
        executor.run_shadow(&candidate, &incumbent),
        ShadowVerdict::Promote { .. }
    ));
    assert_eq!(candidate.calls(), 1);
    assert_eq!(incumbent.calls(), 1);

    let root = TestRoot::new("budget");
    let registry = TripwireRegistry::load_from_vault(root.path()).expect("registry");
    let mut executor = ShadowExecutor::new(registry, replay(1), BudgetHandle::new(0), &clock);
    let before_candidate_calls = candidate.calls();
    let before_incumbent_calls = incumbent.calls();
    assert!(matches!(
        executor.run_shadow(&candidate, &incumbent),
        ShadowVerdict::Revert {
            reason: ShadowRevertReason::BudgetExhausted,
            ..
        }
    ));
    assert_eq!(candidate.calls(), before_candidate_calls);
    assert_eq!(incumbent.calls(), before_incumbent_calls);
}

#[test]
fn missing_and_invalid_metrics_revert_without_partial_query_count() {
    let root = TestRoot::new("missing");
    let clock = FixedClock::new(TEST_TS);
    let registry = TripwireRegistry::load_from_vault(root.path()).expect("registry");
    let candidate = FixedAction::new([
        (TripwireMetric::RecallAtK, 0.95),
        (TripwireMetric::GuardFAR, 0.005),
        (TripwireMetric::GuardFRR, 0.02),
        (TripwireMetric::SearchP99, 120.0),
    ]);
    let incumbent = FixedAction::new(values(0.95, 0.005, 0.02, 120.0, 250.0));
    let mut executor = ShadowExecutor::new(registry, replay(1), BudgetHandle::new(1), &clock);

    match executor.run_shadow(&candidate, &incumbent) {
        ShadowVerdict::Revert { reason, metrics } => {
            assert_eq!(
                reason,
                ShadowRevertReason::MissingMetric {
                    metric: TripwireMetric::IngestP95,
                    side: MetricSide::Candidate,
                }
            );
            assert_eq!(metrics.query_count, 0);
            assert!(metrics.metrics.is_empty());
        }
        other => panic!("expected missing metric revert, got {other:?}"),
    }

    let root = TestRoot::new("invalid");
    let registry = TripwireRegistry::load_from_vault(root.path()).expect("registry");
    let candidate = FixedAction::new(values(0.95, f64::NAN, 0.02, 120.0, 250.0));
    let mut executor = ShadowExecutor::new(registry, replay(1), BudgetHandle::new(1), &clock);
    assert!(matches!(
        executor.run_shadow(&candidate, &incumbent),
        ShadowVerdict::Revert {
            reason: ShadowRevertReason::InvalidMetric {
                metric: TripwireMetric::GuardFAR,
                side: MetricSide::Candidate,
            },
            ..
        }
    ));
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(32))]

    #[test]
    fn dominant_candidate_always_promotes(
        incumbent_recall in 0.90_f64..0.99,
        recall_gain in 0.0_f64..0.01,
        incumbent_far in 0.0001_f64..0.01,
        incumbent_frr in 0.0001_f64..0.05,
        incumbent_search in 1.0_f64..200.0,
        incumbent_ingest in 1.0_f64..500.0,
        upper_ratio in 0.0_f64..1.0,
    ) {
        let root = TestRoot::new("prop");
        let clock = FixedClock::new(TEST_TS);
        let registry = TripwireRegistry::load_from_vault(root.path()).unwrap();
        let candidate_recall = (incumbent_recall + recall_gain).min(1.0);
        let candidate = FixedAction::new(values(
            candidate_recall,
            incumbent_far * upper_ratio,
            incumbent_frr * upper_ratio,
            incumbent_search * upper_ratio,
            incumbent_ingest * upper_ratio,
        ));
        let incumbent = FixedAction::new(values(
            incumbent_recall,
            incumbent_far,
            incumbent_frr,
            incumbent_search,
            incumbent_ingest,
        ));
        let mut executor =
            ShadowExecutor::new(registry, replay(1), BudgetHandle::new(1), &clock);

        let verdict = executor.run_shadow(&candidate, &incumbent);
        prop_assert!(
            matches!(verdict, ShadowVerdict::Promote { .. }),
            "expected promote, got {:?}",
            verdict
        );
    }
}

fn values(
    recall: f64,
    far: f64,
    frr: f64,
    search_p99: f64,
    ingest_p95: f64,
) -> [(TripwireMetric, f64); 5] {
    [
        (TripwireMetric::RecallAtK, recall),
        (TripwireMetric::GuardFAR, far),
        (TripwireMetric::GuardFRR, frr),
        (TripwireMetric::SearchP99, search_p99),
        (TripwireMetric::IngestP95, ingest_p95),
    ]
}

fn replay(count: usize) -> HeldOutReplay {
    HeldOutReplay {
        seed: 42,
        queries: (0..count)
            .map(|query_id| ReplayQuery {
                query_id: query_id as u64,
                query_vector: vec![query_id as f32, 1.0],
                expected_top_k: vec![ReplayAnchor {
                    cx_id: CxId::from_bytes([query_id as u8; 16]),
                    similarity: 0.99,
                }],
            })
            .collect(),
    }
}

fn comparison(snapshot: &MetricSnapshot, metric: TripwireMetric) -> MetricComparison {
    *snapshot
        .metrics
        .iter()
        .find(|comparison| comparison.metric == metric)
        .expect("metric comparison")
}
