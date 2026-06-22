use std::fs;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use calyx_anneal::{
    ABLedgerEvent, ABLedgerWriter, ABRunner, AnnealLedgerAction,
    CALYX_ANNEAL_SOAK_TIME_BUDGET_EXHAUSTED, CALYX_ASTER_CF_UNAVAILABLE, ForgeScopeTuner,
    IndexScopeTuner, LoomScopeTuner, MatPlanConfig, MetricSample, NoopABBudget, NoopSoakStorage,
    SeededSoakProfile, SoakConfig, SoakHarness, SoakMode, SoakReport, SoakStorage,
    TripwireRegistry, check_oscillation,
};
use calyx_aster::vault::AsterVault;
use calyx_core::{CalyxError, FixedClock, Result};
use calyx_forge::AutotuneCache;
use proptest::prelude::*;

mod fsv_support;
use fsv_support::vault_id;

const TEST_TS: u64 = 1_785_500_417;

#[test]
fn seeded_soak_promotes_and_reports_gate() {
    let vault = vault();
    let mut harness = harness(config(1_000, 100), NoopSoakStorage);
    let report = harness.run(&vault).unwrap();

    assert_eq!(report.total_queries, 1_000);
    assert_eq!(report.baseline_p99_ns, 100);
    assert_eq!(report.final_p99_ns, 70);
    assert!(report.p99_reduction >= 0.20);
    assert!(report.recall_final >= report.recall_baseline);
    assert!(!report.oscillation_detected);
    assert!(report.gate_passed);
    assert_eq!(report.promotions.len(), 1);
    assert_eq!(harness.ab_runner.writer.events.len(), 1);
    assert_eq!(
        harness.ab_runner.writer.events[0].action,
        AnnealLedgerAction::AutotunePromote
    );
}

#[test]
fn check_oscillation_flags_last_window_regression() {
    let decreasing = vec![
        sample(1_000, 100),
        sample(2_000, 95),
        sample(3_000, 90),
        sample(4_000, 88),
    ];
    let rising = vec![
        sample(1_000, 100),
        sample(2_000, 90),
        sample(3_000, 89),
        sample(4_000, 99),
    ];

    assert!(!check_oscillation(&decreasing, 3_000));
    assert!(check_oscillation(&rising, 2_000));
}

#[test]
fn zero_queries_emit_zero_report_without_oscillation() {
    let vault = vault();
    let mut harness = harness(config(0, 100), NoopSoakStorage);
    let report = harness.run(&vault).unwrap();

    assert_eq!(report.total_queries, 0);
    assert_eq!(report.baseline_p99_ns, 0);
    assert_eq!(report.final_p99_ns, 0);
    assert_eq!(report.p99_reduction, 0.0);
    assert_eq!(report.recall_final, 0.0);
    assert!(!report.oscillation_detected);
    assert!(report.promotions.is_empty());
}

#[test]
fn equal_latency_profile_reports_no_regression_and_no_oscillation() {
    let vault = vault();
    let profile = SeededSoakProfile {
        final_p99_ns: 100,
        ..SeededSoakProfile::default()
    };
    let mut cfg = config(500, 100);
    cfg.p99_target_reduction = 0.0;
    let mut harness = harness(cfg, NoopSoakStorage).with_seeded_profile(profile);
    let report = harness.run(&vault).unwrap();

    assert_eq!(report.p99_reduction, 0.0);
    assert_eq!(report.recall_final, report.recall_baseline);
    assert!(!report.oscillation_detected);
    assert!(report.gate_passed);
    assert!(report.promotions.is_empty());
}

#[test]
fn live_traffic_constructor_accepts_real_tuner_parts() {
    let vault = vault();
    let cfg = SoakConfig {
        n_queries: 100,
        sample_interval: 50,
        max_runtime_ms: None,
        ..SoakConfig::default()
    };
    let cache = AutotuneCache::load(&temp_cache()).unwrap();
    let mut harness = SoakHarness::live_traffic(
        cfg,
        ForgeScopeTuner::new(cache.clone()),
        IndexScopeTuner::new(cache.clone()),
        LoomScopeTuner::new(cache, MatPlanConfig::default()),
        runner(),
        NoopSoakStorage,
    );

    let report = harness.run(&vault).unwrap();

    assert_eq!(harness.config.mode, SoakMode::LiveTraffic);
    assert!(report.gate_passed);
}

#[test]
fn exhausted_time_budget_returns_partial_report() {
    let vault = vault();
    let mut cfg = config(100, 1);
    cfg.max_runtime_ms = Some(0);
    let mut harness = harness(cfg, NoopSoakStorage);

    let error = harness.run(&vault).unwrap_err();

    assert_eq!(error.code, CALYX_ANNEAL_SOAK_TIME_BUDGET_EXHAUSTED);
    let partial = harness.last_report().expect("partial timeout report");
    assert_eq!(partial.total_queries, 1);
    assert_eq!(partial.samples.len(), 1);
}

#[test]
fn storage_failure_returns_cf_unavailable_and_keeps_partial_report() {
    let vault = vault();
    let storage = FailingStorage::default();
    let mut harness = harness(config(200, 50), storage);

    let error = harness.run(&vault).unwrap_err();

    assert_eq!(error.code, CALYX_ASTER_CF_UNAVAILABLE);
    let partial = harness.last_report().expect("partial report");
    assert_eq!(partial.total_queries, 50);
    assert_eq!(partial.samples.len(), 1);
    assert_eq!(harness.storage.reports.len(), 1);
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(16))]

    #[test]
    fn seeded_better_profile_never_makes_p99_worse(
        n_queries in 100u64..2_000,
        baseline in 80u64..200,
        reduction in 1u64..60,
    ) {
        let vault = vault();
        let final_p99_ns = baseline.saturating_sub(reduction).max(1);
        let profile = SeededSoakProfile {
            baseline_p99_ns: baseline,
            final_p99_ns,
            ..SeededSoakProfile::default()
        };
        let mut cfg = config(n_queries, n_queries);
        cfg.p99_target_reduction = 0.0;
        let mut harness = harness(cfg, NoopSoakStorage).with_seeded_profile(profile);
        let report = harness.run(&vault).unwrap();

        prop_assert!(report.p99_reduction >= 0.0);
        prop_assert!(report.final_p99_ns <= report.baseline_p99_ns);
        prop_assert!(report.recall_final >= report.recall_baseline);
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

#[derive(Default)]
struct FailingStorage {
    reports: Vec<SoakReport>,
}

impl SoakStorage for FailingStorage {
    fn save_sample(&mut self, _run_id: [u8; 32], _sample: &MetricSample) -> Result<()> {
        Err(CalyxError {
            code: CALYX_ASTER_CF_UNAVAILABLE,
            message: "scripted anneal_soak sample failure".to_string(),
            remediation: "restore scripted storage",
        })
    }

    fn save_report(&mut self, _run_id: [u8; 32], report: &SoakReport) -> Result<()> {
        self.reports.push(report.clone());
        Ok(())
    }

    fn scan_rows(&self) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
        Ok(Vec::new())
    }
}

fn harness<S>(config: SoakConfig, storage: S) -> SoakHarness<S, RecordingWriter, NoopABBudget>
where
    S: SoakStorage,
{
    SoakHarness::seeded(
        config,
        AutotuneCache::load(&temp_cache()).unwrap(),
        runner(),
        storage,
    )
}

fn runner() -> ABRunner<RecordingWriter, NoopABBudget> {
    ABRunner::new(
        tripwires(),
        RecordingWriter::default(),
        NoopABBudget::default(),
        Arc::new(FixedClock::new(TEST_TS)),
    )
}

fn config(n_queries: u64, sample_interval: u64) -> SoakConfig {
    SoakConfig {
        n_queries,
        sample_interval,
        max_runtime_ms: None,
        ..SoakConfig::default()
    }
}

fn sample(query_count: u64, p99_ns: u64) -> MetricSample {
    MetricSample {
        p99_ns,
        recall_10: 0.95,
        query_count,
    }
}

fn vault() -> AsterVault<FixedClock> {
    AsterVault::with_clock(vault_id(), b"soak-test".to_vec(), FixedClock::new(TEST_TS))
}

fn temp_cache() -> std::path::PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    std::env::temp_dir().join(format!(
        "calyx-soak-cache-{}-{}.json",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
}

fn tripwires() -> TripwireRegistry {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    let dir = std::env::temp_dir().join(format!(
        "calyx-soak-tripwire-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&dir);
    let registry = TripwireRegistry::load_from_vault(&dir).unwrap();
    let _ = fs::remove_dir_all(&dir);
    registry
}
