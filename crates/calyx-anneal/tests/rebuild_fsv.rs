use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use calyx_anneal::{
    AnnealLedger, AnnealSubstrate, ArtifactPtr, AsterAnnealLedgerStore, AsterHealthStore,
    AsterRollbackStorage, BudgetConfig, BudgetEnforcer, BudgetHandle, BudgetProbe,
    BudgetProbeSample, ChecksumDetector, ChecksumEntry, ComponentKind, DegradeRegistry,
    FaultDetector, FaultMonitor, RebuildOutcome, RebuildPriority, RebuildScheduler, RebuildTarget,
    RollbackStore, decode_health_value,
};
use calyx_aster::cf::{ColumnFamily, base_key, slot_key};
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{CxId, FixedClock, SlotId, SystemClock};
use calyx_ledger::{ActorId, EntryKind, LedgerAppender, decode as decode_ledger};
use serde_json::{Value, json};

mod fsv_support;
use fsv_support::vault_id;

const TEST_TS: u64 = 1_785_600_902;

#[ignore = "manual FSV for #402 rebuild scheduler SoT readback"]
#[test]
fn ph44_rebuild_scheduler_manual_fsv() {
    let root = reset_dir(&fsv_root().join(format!("issue402-{}", std::process::id())));
    let vault_dir = reset_dir(&root.join("vault"));
    let artifact_dir = reset_dir(&root.join("artifacts"));
    let corrupt_ann = artifact_dir.join("ann-slot0-corrupt.bin");
    fs::write(&corrupt_ann, b"corrupt-hnsw-index-before").unwrap();

    let vault = AsterVault::new_durable(
        &vault_dir,
        vault_id(),
        b"issue402-rebuild-fsv-salt".to_vec(),
        VaultOptions::default(),
    )
    .unwrap();
    write_source_rows(&vault);
    vault.flush().unwrap();

    let clock = FixedClock::new(TEST_TS);
    let mut registry = DegradeRegistry::open(Arc::new(clock), AsterHealthStore::new(&vault))
        .expect("open health registry");
    let mut substrate = substrate(&clock, &vault, &vault_dir, budget_config(1.0));
    let target = RebuildTarget::AnnIndex {
        slot_id: SlotId::new(0),
    };
    let source_before = source_readback(&vault);

    let empty_edge = run_empty_edge(&clock, &vault, &artifact_dir, &mut registry, &mut substrate);
    let skip_edge = run_skip_edge(&clock, &vault, &artifact_dir, &mut registry, &mut substrate);

    let detector = ChecksumDetector::new(
        vec![(
            ComponentKind::ann_index(SlotId::new(0)),
            ChecksumEntry::new(corrupt_ann.clone(), [0xff; 32]),
        )],
        Arc::new(clock),
    );
    let mut monitor = FaultMonitor::new(
        vec![Box::new(detector) as Box<dyn FaultDetector<_>>],
        BudgetHandle::new(8),
        100,
    );
    let faults = monitor
        .run_once(&mut registry, &mut substrate.ledger)
        .expect("fault monitor marks degraded");
    let after_fault_health = health_rows(&vault);
    assert!(matches!(
        registry.health(&target.component()),
        calyx_anneal::ComponentHealth::Degraded { .. }
    ));

    let budget_edge = run_budget_edge(&clock, &vault, &artifact_dir, &mut registry);
    substrate
        .rollback
        .install_live_ptr(
            target.artifact_key(),
            ArtifactPtr::HnswGraphPath(corrupt_ann.to_string_lossy().into_owned()),
        )
        .unwrap();
    let mut scheduler = RebuildScheduler::new(&clock, &vault, &artifact_dir);
    scheduler.enqueue(target.clone(), RebuildPriority::HIGH);
    let outcome = scheduler
        .run_next(&mut registry, &mut substrate)
        .expect("rebuild succeeds");
    let RebuildOutcome::Completed { new_ptr, .. } = &outcome else {
        panic!("expected completed rebuild");
    };
    assert_eq!(
        registry.health(&target.component()),
        &calyx_anneal::ComponentHealth::Ok
    );
    vault.flush().unwrap();

    let source_after = source_readback(&vault);
    assert_eq!(source_before, source_after);
    let artifact_readback = artifact_readback(new_ptr);
    let readback = json!({
        "source_of_truth": "Aster base/slot CF bytes, anneal_health CF, anneal_rollback CF, ledger CF, WAL, rebuilt artifact file",
        "vault": vault_dir,
        "artifact_dir": artifact_dir,
        "target": target,
        "faults": faults,
        "empty_queue_edge": empty_edge,
        "not_degraded_edge": skip_edge,
        "budget_exhausted_edge": budget_edge,
        "after_fault_health": after_fault_health,
        "outcome": outcome,
        "after_rebuild_health": health_rows(&vault),
        "source_rows_before": source_before,
        "source_rows_after": source_after,
        "rollback_rows": cf_rows(&vault, ColumnFamily::AnnealRollback),
        "ledger_rows": ledger_rows(&vault),
        "artifact_readback": artifact_readback,
        "wal_files": wal_files(&vault_dir),
    });
    let path = root.join("ph44-rebuild-readback.json");
    fs::write(&path, serde_json::to_vec_pretty(&readback).unwrap()).unwrap();
    println!("PH44_REBUILD_FSV {}", path.display());
}

fn run_empty_edge(
    clock: &FixedClock,
    vault: &AsterVault,
    artifact_dir: &Path,
    registry: &mut DegradeRegistry<AsterHealthStore<'_, SystemClock>>,
    substrate: &mut FsvSubstrate<'_>,
) -> Value {
    let before = health_rows(vault);
    let mut scheduler = RebuildScheduler::new(clock, vault, artifact_dir);
    let outcome = scheduler.run_next(registry, substrate).unwrap();
    json!({"before_health": before, "outcome": outcome, "after_health": health_rows(vault)})
}

fn run_skip_edge(
    clock: &FixedClock,
    vault: &AsterVault,
    artifact_dir: &Path,
    registry: &mut DegradeRegistry<AsterHealthStore<'_, SystemClock>>,
    substrate: &mut FsvSubstrate<'_>,
) -> Value {
    let before = health_rows(vault);
    let mut scheduler = RebuildScheduler::new(clock, vault, artifact_dir);
    scheduler.enqueue(
        RebuildTarget::GuardProfile {
            slot_id: SlotId::new(99),
        },
        RebuildPriority::LOW,
    );
    let outcome = scheduler.run_next(registry, substrate).unwrap();
    json!({"before_health": before, "outcome": outcome, "after_health": health_rows(vault)})
}

fn run_budget_edge(
    clock: &FixedClock,
    vault: &AsterVault,
    artifact_dir: &Path,
    registry: &mut DegradeRegistry<AsterHealthStore<'_, SystemClock>>,
) -> Value {
    let target = RebuildTarget::AnnIndex {
        slot_id: SlotId::new(0),
    };
    let before = health_rows(vault);
    let mut zero_budget = substrate(clock, vault, artifact_dir, budget_config(0.0));
    let mut scheduler = RebuildScheduler::new(clock, vault, artifact_dir);
    scheduler.enqueue(target, RebuildPriority::HIGH);
    let outcome = scheduler.run_next(registry, &mut zero_budget).unwrap();
    json!({
        "before_health": before,
        "outcome": outcome,
        "after_health": health_rows(vault),
        "pending_after": scheduler.pending_len(),
    })
}

type FsvSubstrate<'a> = AnnealSubstrate<
    'a,
    AsterRollbackStorage<'a, SystemClock>,
    AsterAnnealLedgerStore<'a, SystemClock>,
    FixedClock,
    ScriptedProbe,
>;

fn substrate<'a>(
    clock: &'a FixedClock,
    vault: &'a AsterVault,
    vault_dir: &Path,
    config: BudgetConfig,
) -> FsvSubstrate<'a> {
    let rollback = RollbackStore::open(clock, 402, AsterRollbackStorage::new(vault)).unwrap();
    let appender = LedgerAppender::open(AsterAnnealLedgerStore::new(vault), *clock).unwrap();
    let ledger = AnnealLedger::new(
        appender,
        ActorId::Service("calyx-anneal-rebuild-fsv".to_string()),
    )
    .unwrap();
    let budget = BudgetEnforcer::with_probe(config, clock, ScriptedProbe).unwrap();
    AnnealSubstrate::new(
        tripwires(vault_dir),
        replay(),
        rollback,
        ledger,
        budget,
        clock,
    )
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

fn write_source_rows(vault: &AsterVault) {
    let cx = CxId::from_bytes([0x42; 16]);
    vault
        .write_cf_batch([
            (
                ColumnFamily::Base,
                base_key(cx),
                b"issue402-base-row".to_vec(),
            ),
            (
                ColumnFamily::slot(SlotId::new(0)),
                slot_key(cx),
                b"issue402-slot0-row".to_vec(),
            ),
        ])
        .unwrap();
}

fn source_readback(vault: &AsterVault) -> Value {
    json!({
        "base": cf_rows(vault, ColumnFamily::Base),
        "slot_00": cf_rows(vault, ColumnFamily::slot(SlotId::new(0))),
    })
}

fn cf_rows(vault: &AsterVault, cf: ColumnFamily) -> Vec<Value> {
    vault
        .scan_cf_at(vault.latest_seq(), cf)
        .unwrap()
        .into_iter()
        .map(|(key, value)| {
            json!({
                "cf": cf.name(),
                "key_hex": hex(&key),
                "value_len": value.len(),
                "value_b3": hex(blake3::hash(&value).as_bytes()),
                "value_ascii": ascii(&value),
            })
        })
        .collect()
}

fn health_rows(vault: &AsterVault) -> Vec<Value> {
    vault
        .scan_cf_at(vault.latest_seq(), ColumnFamily::AnnealHealth)
        .unwrap()
        .into_iter()
        .map(|(key, value)| {
            let decoded = decode_health_value(&value).unwrap();
            json!({"key_hex": hex(&key), "decoded": decoded})
        })
        .collect()
}

fn ledger_rows(vault: &AsterVault) -> Vec<Value> {
    let mut rows = BTreeMap::new();
    for (key, value) in vault
        .scan_cf_at(vault.latest_seq(), ColumnFamily::Ledger)
        .unwrap()
    {
        let entry = decode_ledger(&value).unwrap();
        assert_eq!(entry.kind, EntryKind::Anneal);
        rows.insert(
            entry.seq,
            json!({
                "key_hex": hex(&key),
                "entry_hash": hex(&entry.entry_hash),
                "payload": serde_json::from_slice::<Value>(&entry.payload).unwrap(),
            }),
        );
    }
    rows.into_values().collect()
}

fn artifact_readback(ptr: &ArtifactPtr) -> Value {
    let ArtifactPtr::HnswGraphPath(path) = ptr else {
        panic!("expected hnsw path");
    };
    let bytes = fs::read(path).unwrap();
    json!({
        "path": path,
        "len": bytes.len(),
        "b3": hex(blake3::hash(&bytes).as_bytes()),
        "text": String::from_utf8(bytes).unwrap(),
    })
}

fn wal_files(vault: &Path) -> Vec<String> {
    let wal_dir = vault.join("wal");
    let mut files = fs::read_dir(wal_dir)
        .unwrap()
        .map(|entry| entry.unwrap().path().display().to_string())
        .collect::<Vec<_>>();
    files.sort();
    files
}

fn tripwires(vault: &Path) -> calyx_anneal::TripwireRegistry {
    let mut registry = calyx_anneal::TripwireRegistry::load_from_vault(vault).unwrap();
    registry
        .set_tripwire(calyx_anneal::TripwireMetric::RecallAtK, 0.90, 0.0)
        .unwrap();
    registry
}

fn replay() -> calyx_anneal::HeldOutReplay {
    calyx_anneal::HeldOutReplay {
        queries: vec![calyx_anneal::ReplayQuery {
            query_id: 402,
            query_vector: vec![1.0, 0.0],
            expected_top_k: vec![calyx_anneal::ReplayAnchor {
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

fn reset_dir(path: &Path) -> PathBuf {
    let _ = fs::remove_dir_all(path);
    fs::create_dir_all(path).unwrap();
    path.to_path_buf()
}

fn fsv_root() -> PathBuf {
    env::var_os("CALYX_ISSUE402_FSV_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| env::temp_dir().join("calyx-issue402-rebuild-fsv"))
}

fn ascii(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|byte| {
            if byte.is_ascii_graphic() || *byte == b' ' {
                *byte as char
            } else {
                '.'
            }
        })
        .collect()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
