use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use calyx_anneal::{
    ActionMetricSnapshot, AnnealLedger, AnnealLedgerAction, AnnealSubstrate, BudgetConfig,
    BudgetEnforcer, BudgetHandle, BudgetProbe, BudgetProbeSample,
    CALYX_ANNEAL_PARK_THRESHOLD_NOT_MET, CALYX_ANNEAL_UNPARK_THRESHOLD_NOT_MET,
    CALYX_WARD_RECALIBRATE_FAILED, ComponentHealth, ComponentKind, DegradeRegistry,
    FileWardTauStore, HealthStorage, HeldOutReplay, LensParkOutcome, NewTau, RecalibrationOutcome,
    ReplayAnchor, ReplayQuery, RollbackStorage, RollbackStore, TauDriftEvent, TripwireMetric,
    TripwireRegistry, WardRecalibrate, WardTauStore, park_decayed_lens, trigger_tau_recalibration,
    unpark_lens,
};
use calyx_core::{CalyxError, CxId, FixedClock, LensId, Result, Seq, SlotId};
use calyx_ledger::{ActorId, LedgerAppender, LedgerCfStore, MemoryLedgerStore};

const TEST_TS: u64 = 1_785_500_404;

#[test]
fn tau_drift_promotes_shadow_winner_and_updates_live_tau() {
    let root = TestRoot::new("tau-promote");
    let slot_id = SlotId::new(0);
    let clock = FixedClock::new(TEST_TS);
    let mut substrate = memory_substrate(&clock, budget_config(1.0), MemoryLedgerStore::default());
    let mut registry = memory_registry();
    let guard = ComponentKind::GuardProfile { slot_id };
    registry
        .set_health(
            guard.clone(),
            ComponentHealth::degraded(TEST_TS, "synthetic FAR creep"),
            &mut substrate.ledger,
        )
        .unwrap();
    let mut tau_store = FileWardTauStore::open(root.path()).unwrap();
    tau_store
        .upsert_current(slot_id, 0.70, TEST_TS - 1)
        .unwrap();
    let ward = StaticWard::ok(new_tau(slot_id, 0.82, 0.001, 0.002, metrics(0.96, 0.001)));
    let drift = drift_event(slot_id, 0.70, metrics(0.91, 0.002));

    let outcome = trigger_tau_recalibration(
        &ward,
        &mut tau_store,
        &mut registry,
        slot_id,
        &drift,
        &mut substrate,
    )
    .unwrap();

    assert!(matches!(
        outcome,
        RecalibrationOutcome::Promoted {
            slot_id: promoted_slot,
            prior_tau: 0.70,
            new_tau,
            ..
        } if promoted_slot == slot_id && (new_tau - 0.82).abs() < f32::EPSILON
    ));
    assert_eq!(
        tau_store.current_tau(slot_id).unwrap(),
        Some(0.82),
        "Ward tau store is the live tau SoT"
    );
    assert_eq!(registry.health(&guard), &ComponentHealth::Ok);
    assert!(
        substrate
            .ledger
            .read_recent(16)
            .unwrap()
            .iter()
            .any(|entry| entry.action == AnnealLedgerAction::TauRecalibrated)
    );
}

#[test]
fn tau_drift_reverts_far_regression_and_keeps_incumbent_tau() {
    let root = TestRoot::new("tau-revert");
    let slot_id = SlotId::new(1);
    let clock = FixedClock::new(TEST_TS);
    let mut substrate = memory_substrate(&clock, budget_config(1.0), MemoryLedgerStore::default());
    let mut registry = memory_registry();
    let mut tau_store = FileWardTauStore::open(root.path()).unwrap();
    tau_store
        .upsert_current(slot_id, 0.71, TEST_TS - 1)
        .unwrap();
    let ward = StaticWard::ok(new_tau(slot_id, 0.88, 0.020, 0.002, metrics(0.96, 0.020)));
    let drift = drift_event(slot_id, 0.71, metrics(0.95, 0.001));

    let outcome = trigger_tau_recalibration(
        &ward,
        &mut tau_store,
        &mut registry,
        slot_id,
        &drift,
        &mut substrate,
    )
    .unwrap();

    assert!(matches!(
        outcome,
        RecalibrationOutcome::Reverted {
            slot_id: reverted_slot,
            prior_tau: 0.71,
            candidate_tau,
            ..
        } if reverted_slot == slot_id && (candidate_tau - 0.88).abs() < f32::EPSILON
    ));
    assert_eq!(tau_store.current_tau(slot_id).unwrap(), Some(0.71));
    assert!(
        substrate
            .ledger
            .read_recent(16)
            .unwrap()
            .iter()
            .any(|entry| entry.action == AnnealLedgerAction::TauRecalibrationReverted)
    );
}

#[test]
fn ward_failure_fails_closed_and_records_reverted_event() {
    let root = TestRoot::new("tau-fail-closed");
    let slot_id = SlotId::new(2);
    let clock = FixedClock::new(TEST_TS);
    let mut substrate = memory_substrate(&clock, budget_config(1.0), MemoryLedgerStore::default());
    let mut registry = memory_registry();
    let mut tau_store = FileWardTauStore::open(root.path()).unwrap();
    tau_store
        .upsert_current(slot_id, 0.73, TEST_TS - 1)
        .unwrap();
    let ward = StaticWard::err("CALYX_SYNTHETIC_WARD_DOWN");
    let drift = drift_event(slot_id, 0.73, metrics(0.90, 0.010));

    let error = trigger_tau_recalibration(
        &ward,
        &mut tau_store,
        &mut registry,
        slot_id,
        &drift,
        &mut substrate,
    )
    .unwrap_err();

    assert_eq!(error.code, CALYX_WARD_RECALIBRATE_FAILED);
    assert_eq!(tau_store.current_tau(slot_id).unwrap(), Some(0.73));
    assert!(
        substrate
            .ledger
            .read_recent(16)
            .unwrap()
            .iter()
            .any(|entry| entry.action == AnnealLedgerAction::TauRecalibrationReverted)
    );
}

#[test]
fn park_and_unpark_lens_update_registry_ledger_alerts_and_edges() {
    let root = TestRoot::new("lens-park");
    let clock = FixedClock::new(TEST_TS);
    let mut registry = memory_registry();
    let mut ledger =
        memory_substrate(&clock, budget_config(1.0), MemoryLedgerStore::default()).ledger;
    let lens_id = lens(1);
    let other_lens = lens(2);
    let alerts_path = root.path().join("alerts.jsonl");

    let outcome = park_decayed_lens(
        lens_id,
        0.04,
        &mut registry,
        &mut ledger,
        &clock,
        &alerts_path,
    )
    .unwrap();

    assert_eq!(outcome, LensParkOutcome::Parked { lens_id });
    assert!(matches!(
        registry.health(&ComponentKind::lens_endpoint(lens_id)),
        ComponentHealth::Parked { .. }
    ));
    assert_eq!(
        registry.active_lenses(&[lens_id, other_lens]),
        vec![other_lens]
    );
    let after_first_park = ledger.read_recent(16).unwrap();
    assert_eq!(
        action_count(&after_first_park, AnnealLedgerAction::LensPark),
        1
    );
    let alert_bytes = fs::read_to_string(&alerts_path).unwrap();
    assert!(alert_bytes.contains("\"action\":\"lens_park\""));
    assert!(alert_bytes.contains("0.04"));

    let already = park_decayed_lens(
        lens_id,
        0.03,
        &mut registry,
        &mut ledger,
        &clock,
        &alerts_path,
    )
    .unwrap();
    assert_eq!(already, LensParkOutcome::AlreadyParked { lens_id });
    assert_eq!(
        action_count(
            &ledger.read_recent(16).unwrap(),
            AnnealLedgerAction::LensPark
        ),
        1,
        "idempotent park must not double-write LensPark"
    );

    let park_error = park_decayed_lens(
        other_lens,
        0.06,
        &mut registry,
        &mut ledger,
        &clock,
        &alerts_path,
    )
    .unwrap_err();
    assert_eq!(park_error.code, CALYX_ANNEAL_PARK_THRESHOLD_NOT_MET);
    let unpark_error = unpark_lens(
        lens_id,
        0.03,
        &mut registry,
        &mut ledger,
        &clock,
        &alerts_path,
    )
    .unwrap_err();
    assert_eq!(unpark_error.code, CALYX_ANNEAL_UNPARK_THRESHOLD_NOT_MET);

    let unparked = unpark_lens(
        lens_id,
        0.05,
        &mut registry,
        &mut ledger,
        &clock,
        &alerts_path,
    )
    .unwrap();
    assert_eq!(unparked, LensParkOutcome::Unparked { lens_id });
    assert_eq!(
        registry.health(&ComponentKind::lens_endpoint(lens_id)),
        &ComponentHealth::Ok
    );
    assert_eq!(
        action_count(
            &ledger.read_recent(16).unwrap(),
            AnnealLedgerAction::LensUnpark
        ),
        1
    );
}

#[derive(Clone)]
struct StaticWard {
    response: Result<NewTau>,
}

impl StaticWard {
    fn ok(tau: NewTau) -> Self {
        Self { response: Ok(tau) }
    }

    fn err(code: &'static str) -> Self {
        Self {
            response: Err(CalyxError {
                code,
                message: "synthetic Ward recalibration outage".to_string(),
                remediation: "test fixture",
            }),
        }
    }
}

impl WardRecalibrate for StaticWard {
    fn recalibrate(&self, slot_id: SlotId, snapshot: u64, _budget: BudgetHandle) -> Result<NewTau> {
        assert_eq!(snapshot, 7);
        let response = self.response.clone()?;
        assert_eq!(response.slot_id, slot_id);
        Ok(response)
    }
}

#[derive(Clone, Default)]
struct MemoryHealthStore {
    inner: Arc<Mutex<BTreeMap<Vec<u8>, Vec<u8>>>>,
}

impl HealthStorage for MemoryHealthStore {
    fn put(&self, key: Vec<u8>, value: Vec<u8>) -> Result<Seq> {
        let mut rows = self.inner.lock().unwrap();
        rows.insert(key, value);
        Ok(rows.len() as Seq)
    }

    fn scan(&self) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
        Ok(self
            .inner
            .lock()
            .unwrap()
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect())
    }
}

#[derive(Clone, Default)]
struct MemoryRollbackStorage {
    rows: Arc<Mutex<BTreeMap<Vec<u8>, Vec<u8>>>>,
}

impl RollbackStorage for MemoryRollbackStorage {
    fn put_many(&self, rows: Vec<(Vec<u8>, Vec<u8>)>) -> Result<Seq> {
        let mut inner = self.rows.lock().unwrap();
        for (key, value) in rows {
            inner.insert(key, value);
        }
        Ok(inner.len() as Seq)
    }

    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>> {
        Ok(self.rows.lock().unwrap().get(key).cloned())
    }

    fn scan(&self) -> Result<Vec<(Vec<u8>, Vec<u8>)>> {
        Ok(self
            .rows
            .lock()
            .unwrap()
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect())
    }
}

#[derive(Clone)]
struct ScriptedProbe {
    sample: BudgetProbeSample,
}

impl BudgetProbe for ScriptedProbe {
    fn sample(&self) -> BudgetProbeSample {
        self.sample.clone()
    }
}

fn memory_registry() -> DegradeRegistry<MemoryHealthStore> {
    DegradeRegistry::open(
        Arc::new(FixedClock::new(TEST_TS)),
        MemoryHealthStore::default(),
    )
    .unwrap()
}

fn memory_substrate<'a, L>(
    clock: &'a FixedClock,
    config: BudgetConfig,
    ledger_store: L,
) -> AnnealSubstrate<'a, MemoryRollbackStorage, L, FixedClock, ScriptedProbe>
where
    L: LedgerCfStore,
{
    let tripwire_root = TestRoot::new("tripwire");
    let tripwires = TripwireRegistry::load_from_vault(tripwire_root.path()).unwrap();
    let rollback = RollbackStore::open(clock, 7, MemoryRollbackStorage::default()).unwrap();
    let appender = LedgerAppender::open(ledger_store, *clock).unwrap();
    let ledger = AnnealLedger::new(
        appender,
        ActorId::Service("calyx-anneal-recalibrate-test".to_string()),
    )
    .unwrap();
    let budget = BudgetEnforcer::with_probe(config, clock, scripted_probe()).unwrap();
    AnnealSubstrate::new(tripwires, replay(), rollback, ledger, budget, clock)
}

fn budget_config(cpu_fraction: f64) -> BudgetConfig {
    BudgetConfig {
        cpu_fraction,
        vram_bytes: 1024,
        tick_interval_ms: 100,
    }
}

fn replay() -> HeldOutReplay {
    HeldOutReplay {
        queries: vec![ReplayQuery {
            query_id: 404,
            query_vector: vec![1.0, 0.0],
            expected_top_k: vec![ReplayAnchor {
                cx_id: CxId::from_bytes([4; 16]),
                similarity: 1.0,
            }],
        }],
        seed: 404,
    }
}

fn scripted_probe() -> ScriptedProbe {
    ScriptedProbe {
        sample: BudgetProbeSample {
            cpu_used_fraction: 0.0,
            vram_used_bytes: 0,
            nvml_available: true,
        },
    }
}

fn new_tau(slot_id: SlotId, tau: f32, far: f64, frr: f64, metrics: ActionMetricSnapshot) -> NewTau {
    NewTau::new(slot_id, tau, far, frr, metrics).unwrap()
}

fn drift_event(
    slot_id: SlotId,
    current_tau: f32,
    incumbent_metrics: ActionMetricSnapshot,
) -> TauDriftEvent {
    TauDriftEvent::new(
        slot_id,
        current_tau,
        0.020,
        0.005,
        TEST_TS,
        7,
        incumbent_metrics,
    )
    .unwrap()
}

fn metrics(recall: f64, far: f64) -> ActionMetricSnapshot {
    ActionMetricSnapshot::from_values([
        (TripwireMetric::RecallAtK, recall),
        (TripwireMetric::GuardFAR, far),
        (TripwireMetric::GuardFRR, 0.001),
        (TripwireMetric::SearchP99, 50.0),
        (TripwireMetric::IngestP95, 80.0),
    ])
}

fn lens(seed: u8) -> LensId {
    LensId::from_parts(
        &format!("synthetic-lens-{seed}"),
        &[seed; 32],
        &[seed.wrapping_add(1); 32],
        b"2",
    )
}

fn action_count(entries: &[calyx_anneal::AnnealLedgerEntry], action: AnnealLedgerAction) -> usize {
    entries
        .iter()
        .filter(|entry| entry.action == action)
        .count()
}

struct TestRoot(PathBuf);

impl TestRoot {
    fn new(label: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "calyx-recalibrate-{label}-{}-{}",
            std::process::id(),
            NEXT_ROOT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
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
