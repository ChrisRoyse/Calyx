use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use calyx_anneal::{
    ActionMetricSnapshot, AnnealAction, BudgetHandle, HeldOutReplay, MetricSide, ReplayAnchor,
    ReplayQuery, ShadowExecutor, ShadowRevertReason, ShadowVerdict, TripwireMetric,
    TripwireRegistry, tripwire_config_path,
};
use calyx_core::{CxId, FixedClock};
use serde_json::json;

#[path = "fsv_support/mod.rs"]
mod fsv_support;
use fsv_support::{write_json, write_manifest};

const FSV_TS: u64 = 1_785_500_395;

#[test]
#[ignore = "requires CALYX_ISSUE395_FSV_ROOT in a manual verification run"]
fn issue395_shadow_executor_fsv() {
    let root =
        PathBuf::from(env::var("CALYX_ISSUE395_FSV_ROOT").expect("set CALYX_ISSUE395_FSV_ROOT"));
    fs::create_dir_all(&root).expect("create FSV root");
    let vault = root.join("vault");
    fs::create_dir_all(&vault).expect("create FSV vault");
    let live_pointer = root.join("live-config-pointer.txt");
    fs::write(
        &live_pointer,
        "active_config=incumbent-hnsw-ef64-blake3-demo\n",
    )
    .expect("write live pointer");
    let pointer_before = fs::read_to_string(&live_pointer).expect("read pointer before");

    let clock = FixedClock::new(FSV_TS);
    let revert_verdict = run_shadow(
        &vault,
        replay(2),
        2,
        &clock,
        values(0.80, 0.004, 0.015, 110.0, 240.0),
        values(0.95, 0.006, 0.020, 130.0, 260.0),
    );
    assert!(matches!(
        revert_verdict,
        ShadowVerdict::Revert {
            reason: ShadowRevertReason::TripwireCrossed(TripwireMetric::RecallAtK),
            ..
        }
    ));
    let pointer_after_revert = fs::read_to_string(&live_pointer).expect("read pointer after");
    assert_eq!(pointer_after_revert, pointer_before);

    let promote_verdict = run_shadow(
        &vault,
        replay(3),
        3,
        &clock,
        values(0.97, 0.003, 0.010, 90.0, 200.0),
        values(0.95, 0.006, 0.020, 130.0, 260.0),
    );
    assert!(matches!(promote_verdict, ShadowVerdict::Promote { .. }));

    let empty_edge = edge_empty_replay(&vault, &clock, &live_pointer);
    let budget_edge = edge_budget_zero(&vault, &clock, &live_pointer);
    let single_equal_edge = edge_single_equal(&vault, &clock, &live_pointer);
    let invalid_edge = edge_invalid_metric(&vault, &clock, &live_pointer);

    let pointer_after = fs::read_to_string(&live_pointer).expect("read pointer after edges");
    fs::write(root.join("live-config-pointer-before.txt"), &pointer_before)
        .expect("write pointer before readback");
    fs::write(root.join("live-config-pointer-after.txt"), &pointer_after)
        .expect("write pointer after readback");
    assert_eq!(pointer_after, pointer_before);

    write_json(
        &root.join("shadow-verdicts.json"),
        &json!({
            "surface": "anneal.shadow_executor",
            "source_of_truth": "durable shadow verdict JSON and synthetic live config pointer bytes",
            "vault": vault.display().to_string(),
            "tripwire_config": tripwire_config_path(&vault).display().to_string(),
            "trigger": "run_shadow(candidate recall 0.80, incumbent recall 0.95)",
            "expected": "Revert with TripwireCrossed(RecallAtK) and unchanged live pointer",
            "pointer_before": pointer_before,
            "revert_verdict": revert_verdict,
            "pointer_after_revert": pointer_after_revert,
            "promote_verdict": promote_verdict,
            "edges": [
                empty_edge,
                budget_edge,
                single_equal_edge,
                invalid_edge
            ],
            "pointer_after_all_edges": pointer_after
        }),
    );
    write_manifest(
        &root,
        &[
            root.join("vault/.anneal/tripwire.toml"),
            root.join("live-config-pointer-before.txt"),
            root.join("live-config-pointer-after.txt"),
            root.join("shadow-verdicts.json"),
        ],
    );
}

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

fn run_shadow(
    vault: &Path,
    replay: HeldOutReplay,
    budget: usize,
    clock: &FixedClock,
    candidate_values: [(TripwireMetric, f64); 5],
    incumbent_values: [(TripwireMetric, f64); 5],
) -> ShadowVerdict {
    let registry = TripwireRegistry::load_from_vault(vault).expect("load tripwires");
    let candidate = FixedAction::new(candidate_values);
    let incumbent = FixedAction::new(incumbent_values);
    let mut executor = ShadowExecutor::new(registry, replay, BudgetHandle::new(budget), clock);
    executor.run_shadow(&candidate, &incumbent)
}

fn edge_empty_replay(vault: &Path, clock: &FixedClock, live_pointer: &Path) -> serde_json::Value {
    let before = fs::read_to_string(live_pointer).expect("edge pointer before");
    let registry = TripwireRegistry::load_from_vault(vault).expect("registry");
    let candidate = FixedAction::new(values(0.95, 0.005, 0.020, 120.0, 250.0));
    let incumbent = FixedAction::new(values(0.95, 0.005, 0.020, 120.0, 250.0));
    let mut executor = ShadowExecutor::new(registry, replay(0), BudgetHandle::new(1), clock);
    let before_state = json!({
        "pointer": before,
        "candidate_calls": candidate.calls(),
        "incumbent_calls": incumbent.calls(),
        "budget_remaining": executor.budget.remaining_ticks()
    });
    let verdict = executor.run_shadow(&candidate, &incumbent);
    let after = fs::read_to_string(live_pointer).expect("edge pointer after");
    json!({
        "case": "empty_replay",
        "expected": "InsufficientReplay, zero action calls, pointer unchanged",
        "before": before_state,
        "verdict": verdict,
        "after": {
            "pointer": after,
            "candidate_calls": candidate.calls(),
            "incumbent_calls": incumbent.calls(),
            "budget_remaining": executor.budget.remaining_ticks()
        }
    })
}

fn edge_budget_zero(vault: &Path, clock: &FixedClock, live_pointer: &Path) -> serde_json::Value {
    let before = fs::read_to_string(live_pointer).expect("edge pointer before");
    let registry = TripwireRegistry::load_from_vault(vault).expect("registry");
    let candidate = FixedAction::new(values(0.95, 0.005, 0.020, 120.0, 250.0));
    let incumbent = FixedAction::new(values(0.95, 0.005, 0.020, 120.0, 250.0));
    let mut executor = ShadowExecutor::new(registry, replay(1), BudgetHandle::new(0), clock);
    let before_state = json!({
        "pointer": before,
        "candidate_calls": candidate.calls(),
        "incumbent_calls": incumbent.calls(),
        "budget_remaining": executor.budget.remaining_ticks()
    });
    let verdict = executor.run_shadow(&candidate, &incumbent);
    let after = fs::read_to_string(live_pointer).expect("edge pointer after");
    json!({
        "case": "budget_zero",
        "expected": "BudgetExhausted before any query runs, pointer unchanged",
        "before": before_state,
        "verdict": verdict,
        "after": {
            "pointer": after,
            "candidate_calls": candidate.calls(),
            "incumbent_calls": incumbent.calls(),
            "budget_remaining": executor.budget.remaining_ticks()
        }
    })
}

fn edge_single_equal(vault: &Path, clock: &FixedClock, live_pointer: &Path) -> serde_json::Value {
    let before = fs::read_to_string(live_pointer).expect("edge pointer before");
    let verdict = run_shadow(
        vault,
        replay(1),
        1,
        clock,
        values(0.95, 0.005, 0.020, 120.0, 250.0),
        values(0.95, 0.005, 0.020, 120.0, 250.0),
    );
    let after = fs::read_to_string(live_pointer).expect("edge pointer after");
    json!({
        "case": "single_query_equal",
        "expected": "Promote because equality is no regression and metrics pass",
        "before": {"pointer": before},
        "verdict": verdict,
        "after": {"pointer": after}
    })
}

fn edge_invalid_metric(vault: &Path, clock: &FixedClock, live_pointer: &Path) -> serde_json::Value {
    let before = fs::read_to_string(live_pointer).expect("edge pointer before");
    let registry = TripwireRegistry::load_from_vault(vault).expect("registry");
    let candidate = FixedAction::new(values(0.95, f64::NAN, 0.020, 120.0, 250.0));
    let incumbent = FixedAction::new(values(0.95, 0.005, 0.020, 120.0, 250.0));
    let mut executor = ShadowExecutor::new(registry, replay(1), BudgetHandle::new(1), clock);
    let before_state = json!({
        "pointer": before,
        "candidate_calls": candidate.calls(),
        "incumbent_calls": incumbent.calls(),
        "budget_remaining": executor.budget.remaining_ticks()
    });
    let verdict = executor.run_shadow(&candidate, &incumbent);
    assert!(matches!(
        verdict,
        ShadowVerdict::Revert {
            reason: ShadowRevertReason::InvalidMetric {
                metric: TripwireMetric::GuardFAR,
                side: MetricSide::Candidate,
            },
            ..
        }
    ));
    let after = fs::read_to_string(live_pointer).expect("edge pointer after");
    json!({
        "case": "invalid_metric_nan",
        "expected": "InvalidMetric candidate guard_far and pointer unchanged",
        "before": before_state,
        "verdict": verdict,
        "after": {
            "pointer": after,
            "candidate_calls": candidate.calls(),
            "incumbent_calls": incumbent.calls(),
            "budget_remaining": executor.budget.remaining_ticks()
        }
    })
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
        seed: 395,
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
