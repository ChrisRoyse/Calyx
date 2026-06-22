use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use calyx_anneal::{
    AnnIndexRebuilder, AnnealLedger, AnnealLedgerAction, AnnealSubstrate, ArtifactPtr,
    AsterHealthStore, AsterRebuildSource, BudgetConfig, BudgetEnforcer, BudgetProbe,
    BudgetProbeSample, CALYX_ANNEAL_REBUILD_SOURCE_VIOLATION, CALYX_ASTER_SNAPSHOT_UNAVAILABLE,
    ComponentHealth, DegradeRegistry, HeldOutReplay, RebuildOutcome, RebuildPriority,
    RebuildScheduler, RebuildTarget, Rebuilder, ReplayAnchor, ReplayQuery, RollbackStorage,
    RollbackStore, TripwireMetric, TripwireRegistry,
};
use calyx_aster::cf::{ColumnFamily, base_key, slot_key};
use calyx_aster::vault::AsterVault;
use calyx_core::{CalyxError, CxId, FixedClock, Result, Seq, SlotId};
use calyx_ledger::{ActorId, LedgerAppender, LedgerCfStore, MemoryLedgerStore};
use proptest::prelude::*;

mod fsv_support;
use fsv_support::vault_id;

const TEST_TS: u64 = 1_785_600_402;

#[test]
fn ann_rebuild_promotes_pointer_confirms_health_and_writes_rebuild_ledger() {
    let clock = FixedClock::new(TEST_TS);
    let vault = source_vault(clock);
    write_source_rows(&vault);
    let mut registry = registry(&vault, clock);
    let mut substrate = memory_substrate(&clock, budget_config(1.0), MemoryLedgerStore::default());
    let target = ann_target();
    let prior = prior_ann_ptr();
    mark_degraded(&mut registry, &mut substrate.ledger, &target);
    substrate
        .rollback
        .install_live_ptr(target.artifact_key(), prior.clone())
        .unwrap();

    let mut scheduler = RebuildScheduler::new(&clock, &vault, temp_dir("ann-success"));
    scheduler.enqueue(target.clone(), RebuildPriority::HIGH);
    let outcome = scheduler.run_next(&mut registry, &mut substrate).unwrap();

    let RebuildOutcome::Completed {
        change_id,
        prior_ptr,
        new_ptr,
    } = outcome
    else {
        panic!("expected completed rebuild");
    };
    assert_eq!(prior_ptr, prior);
    assert_ne!(new_ptr, prior_ptr);
    assert_eq!(registry.health(&target.component()), &ComponentHealth::Ok);
    assert_eq!(
        substrate.rollback.live_ptr(&target.artifact_key()).unwrap(),
        Some(new_ptr.clone())
    );
    assert!(matches!(new_ptr, ArtifactPtr::HnswGraphPath(_)));
    if let ArtifactPtr::HnswGraphPath(path) = &new_ptr {
        let bytes = fs::read(path).unwrap();
        let text = String::from_utf8(bytes).unwrap();
        assert!(text.contains("ann_index_v1"));
        assert!(text.contains("slot"));
    }
    let recent = substrate.ledger.read_recent(8).unwrap();
    assert!(
        recent
            .iter()
            .any(|entry| entry.action == AnnealLedgerAction::Rebuild
                && entry.change_id == change_id
                && entry.description == "rebuild completed")
    );
}

#[test]
fn source_violation_fails_closed_and_leaves_component_degraded() {
    let clock = FixedClock::new(TEST_TS);
    let vault = source_vault(clock);
    write_source_rows(&vault);
    let source = AsterRebuildSource::new(&vault);
    let mut registry = registry(&vault, clock);
    let mut substrate = memory_substrate(&clock, budget_config(1.0), MemoryLedgerStore::default());
    let target = ann_target();
    mark_degraded(&mut registry, &mut substrate.ledger, &target);
    substrate
        .rollback
        .install_live_ptr(target.artifact_key(), prior_ann_ptr())
        .unwrap();
    let ann = AnnIndexRebuilder::new(source, temp_dir("source-violation"))
        .with_derived_probe_for_test(ColumnFamily::Ledger);
    let mut scheduler = RebuildScheduler::with_rebuilders(
        &clock,
        source,
        Box::new(ann),
        Box::new(NoopRebuilder),
        Box::new(NoopRebuilder),
    );

    scheduler.enqueue(target.clone(), RebuildPriority::HIGH);
    let outcome = scheduler.run_next(&mut registry, &mut substrate).unwrap();

    assert!(matches!(
        outcome,
        RebuildOutcome::Failed {
            ref reason_code,
            ..
        } if reason_code == CALYX_ANNEAL_REBUILD_SOURCE_VIOLATION
    ));
    assert!(matches!(
        registry.health(&target.component()),
        ComponentHealth::Degraded { .. }
    ));
    let recent = substrate.ledger.read_recent(2).unwrap();
    assert_eq!(recent.last().unwrap().action, AnnealLedgerAction::Rebuild);
    assert!(
        recent
            .last()
            .unwrap()
            .description
            .contains(CALYX_ANNEAL_REBUILD_SOURCE_VIOLATION)
    );
}

#[test]
fn queue_edges_empty_not_degraded_and_budget_exhausted() {
    let clock = FixedClock::new(TEST_TS);
    let vault = source_vault(clock);
    write_source_rows(&vault);
    let mut registry = registry(&vault, clock);
    let target = ann_target();
    let mut scheduler = RebuildScheduler::new(&clock, &vault, temp_dir("edges"));
    let mut substrate = memory_substrate(&clock, budget_config(1.0), MemoryLedgerStore::default());

    assert_eq!(
        scheduler.run_next(&mut registry, &mut substrate).unwrap(),
        RebuildOutcome::NothingQueued
    );
    scheduler.enqueue(target.clone(), RebuildPriority::NORMAL);
    assert!(matches!(
        scheduler.run_next(&mut registry, &mut substrate).unwrap(),
        RebuildOutcome::SkippedNotDegraded { .. }
    ));

    mark_degraded(&mut registry, &mut substrate.ledger, &target);
    substrate
        .rollback
        .install_live_ptr(target.artifact_key(), prior_ann_ptr())
        .unwrap();
    let mut exhausted = memory_substrate(&clock, budget_config(0.0), MemoryLedgerStore::default());
    exhausted
        .rollback
        .install_live_ptr(target.artifact_key(), prior_ann_ptr())
        .unwrap();
    scheduler.enqueue(target.clone(), RebuildPriority::HIGH);
    assert!(matches!(
        scheduler.run_next(&mut registry, &mut exhausted).unwrap(),
        RebuildOutcome::BudgetExhausted { .. }
    ));
    assert_eq!(scheduler.pending_len(), 1);
    assert!(matches!(
        registry.health(&target.component()),
        ComponentHealth::Degraded { .. }
    ));
}

#[test]
fn snapshot_failure_records_rebuild_failure() {
    let clock = FixedClock::new(TEST_TS);
    let vault = source_vault(clock);
    let source = AsterRebuildSource::new(&vault);
    let mut registry = registry(&vault, clock);
    let mut substrate = memory_substrate(&clock, budget_config(1.0), MemoryLedgerStore::default());
    let target = ann_target();
    mark_degraded(&mut registry, &mut substrate.ledger, &target);
    substrate
        .rollback
        .install_live_ptr(target.artifact_key(), prior_ann_ptr())
        .unwrap();
    let mut scheduler = RebuildScheduler::with_rebuilders(
        &clock,
        source,
        Box::new(SnapshotFailRebuilder),
        Box::new(NoopRebuilder),
        Box::new(NoopRebuilder),
    );

    scheduler.enqueue(target.clone(), RebuildPriority::HIGH);
    let outcome = scheduler.run_next(&mut registry, &mut substrate).unwrap();

    assert!(matches!(
        outcome,
        RebuildOutcome::Failed {
            ref reason_code,
            ..
        } if reason_code == CALYX_ASTER_SNAPSHOT_UNAVAILABLE
    ));
    assert!(matches!(
        registry.health(&target.component()),
        ComponentHealth::Degraded { .. }
    ));
    assert!(
        substrate
            .ledger
            .read_recent(2)
            .unwrap()
            .last()
            .unwrap()
            .description
            .contains(CALYX_ASTER_SNAPSHOT_UNAVAILABLE)
    );
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(16))]

    #[test]
    fn successful_runs_leave_component_ok(ops in prop::collection::vec(any::<bool>(), 1..12)) {
        let clock = FixedClock::new(TEST_TS);
        let vault = source_vault(clock);
        write_source_rows(&vault);
        let mut registry = registry(&vault, clock);
        let mut substrate = memory_substrate(&clock, budget_config(1.0), MemoryLedgerStore::default());
        let target = ann_target();
        let mut scheduler = RebuildScheduler::new(&clock, &vault, temp_dir("prop"));

        for should_degrade in ops {
            if should_degrade {
                mark_degraded(&mut registry, &mut substrate.ledger, &target);
                substrate
                    .rollback
                    .install_live_ptr(target.artifact_key(), prior_ann_ptr())
                    .unwrap();
            }
            scheduler.enqueue(target.clone(), RebuildPriority::NORMAL);
            let outcome = scheduler.run_next(&mut registry, &mut substrate).unwrap();
            if matches!(outcome, RebuildOutcome::Completed { .. }) {
                prop_assert_eq!(registry.health(&target.component()), &ComponentHealth::Ok);
            }
        }
    }
}

struct NoopRebuilder;

impl Rebuilder for NoopRebuilder {
    fn rebuild(
        &self,
        _target: &RebuildTarget,
        _snapshot: calyx_anneal::MvccSnapshot,
        _budget: &mut calyx_anneal::BudgetHandle,
    ) -> Result<ArtifactPtr> {
        Ok(ArtifactPtr::ConfigCacheKeyHash([7; 32]))
    }
}

struct SnapshotFailRebuilder;

impl Rebuilder for SnapshotFailRebuilder {
    fn rebuild(
        &self,
        _target: &RebuildTarget,
        _snapshot: calyx_anneal::MvccSnapshot,
        _budget: &mut calyx_anneal::BudgetHandle,
    ) -> Result<ArtifactPtr> {
        Err(CalyxError {
            code: CALYX_ASTER_SNAPSHOT_UNAVAILABLE,
            message: "injected snapshot outage".to_string(),
            remediation: "retry rebuild against a fresh Aster MVCC snapshot",
        })
    }
}

fn source_vault(clock: FixedClock) -> AsterVault<FixedClock> {
    AsterVault::with_clock(vault_id(), b"issue402-rebuild-salt".to_vec(), clock)
}

fn write_source_rows(vault: &AsterVault<FixedClock>) {
    let cx = CxId::from_bytes([4; 16]);
    vault
        .write_cf_batch([
            (
                ColumnFamily::Base,
                base_key(cx),
                b"base-row-issue402".to_vec(),
            ),
            (
                ColumnFamily::slot(SlotId::new(0)),
                slot_key(cx),
                b"slot-row-issue402".to_vec(),
            ),
        ])
        .unwrap();
}

fn registry(
    vault: &AsterVault<FixedClock>,
    clock: FixedClock,
) -> DegradeRegistry<AsterHealthStore<'_, FixedClock>> {
    DegradeRegistry::open(Arc::new(clock), AsterHealthStore::new(vault)).unwrap()
}

fn mark_degraded<L, C>(
    registry: &mut DegradeRegistry<AsterHealthStore<'_, FixedClock>>,
    ledger: &mut calyx_anneal::AnnealLedger<L, C>,
    target: &RebuildTarget,
) where
    L: calyx_ledger::LedgerCfStore,
    C: calyx_core::Clock,
{
    registry
        .set_health(
            target.component(),
            ComponentHealth::degraded(TEST_TS, "synthetic corrupt derived artifact"),
            ledger,
        )
        .unwrap();
}

fn ann_target() -> RebuildTarget {
    RebuildTarget::AnnIndex {
        slot_id: SlotId::new(0),
    }
}

fn prior_ann_ptr() -> ArtifactPtr {
    ArtifactPtr::HnswGraphPath("corrupt-hnsw-before-rebuild.bin".to_string())
}

fn temp_dir(label: &str) -> PathBuf {
    let path = env::temp_dir().join(format!(
        "calyx-issue402-rebuild-{label}-{}-{}",
        std::process::id(),
        NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(&path).unwrap();
    path
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
struct ScriptedProbe;

impl BudgetProbe for ScriptedProbe {
    fn sample(&self) -> BudgetProbeSample {
        BudgetProbeSample {
            cpu_used_fraction: 0.0,
            vram_used_bytes: 0,
            nvml_available: true,
        }
    }
}

fn memory_substrate<'a, L>(
    clock: &'a FixedClock,
    config: BudgetConfig,
    ledger_store: L,
) -> AnnealSubstrate<'a, MemoryRollbackStorage, L, FixedClock, ScriptedProbe>
where
    L: LedgerCfStore,
{
    let rollback = RollbackStore::open(clock, 402, MemoryRollbackStorage::default()).unwrap();
    let appender = LedgerAppender::open(ledger_store, *clock).unwrap();
    let ledger = AnnealLedger::new(
        appender,
        ActorId::Service("calyx-anneal-rebuild-test".to_string()),
    )
    .unwrap();
    let budget = BudgetEnforcer::with_probe(config, clock, ScriptedProbe).unwrap();
    AnnealSubstrate::new(tripwires(), replay(), rollback, ledger, budget, clock)
}

fn tripwires() -> TripwireRegistry {
    let mut registry = TripwireRegistry::load_from_vault(temp_dir("tripwire")).unwrap();
    registry
        .set_tripwire(TripwireMetric::RecallAtK, 0.90, 0.0)
        .unwrap();
    registry
}

fn replay() -> HeldOutReplay {
    HeldOutReplay {
        queries: vec![ReplayQuery {
            query_id: 1,
            query_vector: vec![1.0, 0.0],
            expected_top_k: vec![ReplayAnchor {
                cx_id: CxId::from_bytes([1; 16]),
                similarity: 1.0,
            }],
        }],
        seed: 402,
    }
}

fn budget_config(cpu_fraction: f64) -> BudgetConfig {
    BudgetConfig {
        cpu_fraction,
        vram_bytes: 1024,
        tick_interval_ms: 100,
    }
}

static NEXT_TEMP: AtomicUsize = AtomicUsize::new(0);
