use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use calyx_anneal::{
    ActionMetricSnapshot, AnnealAction, AnnealLedger, AnnealSubstrate, ArtifactKey, ArtifactPtr,
    AsterAnnealLedgerStore, AsterRollbackStorage, BudgetConfig, BudgetEnforcer, BudgetProbe,
    BudgetProbeSample, CALYX_ASTER_CF_UNAVAILABLE, HeldOutReplay, ReplayAnchor, ReplayQuery,
    RollbackStorage, RollbackStore, TripwireMetric, TripwireRegistry,
};
use calyx_aster::cf::{ColumnFamily, ledger_key};
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{CalyxError, CxId, FixedClock, Result, Seq, SystemClock};
use calyx_ledger::{
    ActorId, EntryKind, LedgerAppender, LedgerCfStore, LedgerRow, decode as decode_ledger,
};
use serde_json::{Value, json};

#[allow(clippy::duplicate_mod)]
#[path = "../fsv_support/mod.rs"]
mod fsv_support;
#[allow(unused_imports)]
pub use fsv_support::write_json;

pub const TEST_TS: u64 = 1_785_500_399;
const PRIOR: [u8; 32] = [0x11; 32];
const CANDIDATE: [u8; 32] = [0x22; 32];

#[derive(Clone)]
pub struct FixedAction {
    recall: f64,
}

impl AnnealAction for FixedAction {
    fn apply_shadow(&self, _query: &ReplayQuery) -> ActionMetricSnapshot {
        ActionMetricSnapshot::from_values([
            (TripwireMetric::RecallAtK, self.recall),
            (TripwireMetric::GuardFAR, 0.001),
            (TripwireMetric::GuardFRR, 0.001),
            (TripwireMetric::SearchP99, 50.0),
            (TripwireMetric::IngestP95, 80.0),
        ])
    }
}

#[derive(Clone, Default)]
pub struct MemoryRollbackStorage {
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

pub struct FailingLedgerStore;

impl LedgerCfStore for FailingLedgerStore {
    fn scan(&self) -> Result<Vec<LedgerRow>> {
        Ok(Vec::new())
    }

    fn put_new(&mut self, _seq: u64, _bytes: &[u8]) -> Result<()> {
        Err(CalyxError {
            code: CALYX_ASTER_CF_UNAVAILABLE,
            message: "injected ledger outage".to_string(),
            remediation: "restore ledger CF",
        })
    }
}

#[derive(Clone)]
pub struct ScriptedProbe {
    sample: BudgetProbeSample,
}

impl BudgetProbe for ScriptedProbe {
    fn sample(&self) -> BudgetProbeSample {
        self.sample.clone()
    }
}

pub type DurableSubstrate<'a> = AnnealSubstrate<
    'a,
    AsterRollbackStorage<'a, SystemClock>,
    AsterAnnealLedgerStore<'a, SystemClock>,
    FixedClock,
    ScriptedProbe,
>;

pub fn memory_substrate<'a, L>(
    clock: &'a FixedClock,
    config: BudgetConfig,
    ledger_store: L,
) -> AnnealSubstrate<'a, MemoryRollbackStorage, L, FixedClock, ScriptedProbe>
where
    L: LedgerCfStore,
{
    let tripwire_root = TestRoot::new("tripwire");
    let tripwires = tripwires(tripwire_root.path());
    let rollback = RollbackStore::open(clock, 7, MemoryRollbackStorage::default()).unwrap();
    let appender = LedgerAppender::open(ledger_store, *clock).unwrap();
    let ledger = AnnealLedger::new(
        appender,
        ActorId::Service("calyx-anneal-integration-test".to_string()),
    )
    .unwrap();
    let budget = BudgetEnforcer::with_probe(config, clock, scripted_probe()).unwrap();
    AnnealSubstrate::new(tripwires, replay(), rollback, ledger, budget, clock)
}

pub fn durable_substrate<'a>(
    clock: &'a FixedClock,
    vault: &'a AsterVault,
    vault_dir: &Path,
) -> DurableSubstrate<'a> {
    durable_substrate_with_budget(clock, vault, vault_dir, budget_config(1.0))
}

pub fn durable_substrate_with_budget<'a>(
    clock: &'a FixedClock,
    vault: &'a AsterVault,
    vault_dir: &Path,
    config: BudgetConfig,
) -> DurableSubstrate<'a> {
    let rollback = RollbackStore::open(clock, 9, AsterRollbackStorage::new(vault)).unwrap();
    let appender = LedgerAppender::open(AsterAnnealLedgerStore::new(vault), *clock).unwrap();
    let ledger =
        AnnealLedger::new(appender, ActorId::Service("calyx-anneal-fsv".to_string())).unwrap();
    let budget = BudgetEnforcer::with_probe(config, clock, scripted_probe()).unwrap();
    AnnealSubstrate::new(
        tripwires(vault_dir),
        replay(),
        rollback,
        ledger,
        budget,
        clock,
    )
}

pub fn open_durable_vault(root: &Path, label: &str) -> (PathBuf, AsterVault) {
    let vault_dir = root.join(label);
    fs::create_dir_all(&vault_dir).unwrap();
    let salt = format!("issue399-{label}-salt").into_bytes();
    let vault = AsterVault::new_durable(
        &vault_dir,
        fsv_support::vault_id(),
        salt,
        VaultOptions::default(),
    )
    .expect("open durable vault");
    (vault_dir, vault)
}

pub fn install_prior<S: RollbackStorage>(rollback: &RollbackStore<'_, S>) {
    rollback
        .install_live_ptr(artifact_key(), prior_ptr())
        .expect("install prior");
}

pub fn read_ledger_rows(vault: &AsterVault) -> Vec<Value> {
    vault
        .scan_cf_at(vault.latest_seq(), ColumnFamily::Ledger)
        .unwrap()
        .into_iter()
        .map(|(key, bytes)| {
            let entry = decode_ledger(&bytes).unwrap();
            assert_eq!(entry.kind, EntryKind::Anneal);
            assert_eq!(key, ledger_key(entry.seq));
            json!({
                "seq": entry.seq,
                "key_hex": hex(&key),
                "kind": entry.kind.as_str(),
                "entry_hash": hex(&entry.entry_hash),
                "payload_json": serde_json::from_slice::<Value>(&entry.payload).unwrap()
            })
        })
        .collect()
}

pub fn read_rollback_rows(vault: &AsterVault) -> Vec<Value> {
    vault
        .scan_cf_at(vault.latest_seq(), ColumnFamily::AnnealRollback)
        .unwrap()
        .into_iter()
        .map(|(key, bytes)| {
            json!({
                "key_hex": hex(&key),
                "key_ascii": ascii_preview(&key),
                "value_len": bytes.len(),
                "value_hex": hex(&bytes)
            })
        })
        .collect()
}

pub fn reset_dir(dir: &Path) {
    let _ = fs::remove_dir_all(dir);
    fs::create_dir_all(dir).unwrap();
}

pub fn budget_config(cpu_fraction: f64) -> BudgetConfig {
    BudgetConfig {
        cpu_fraction,
        vram_bytes: 1024,
        tick_interval_ms: 100,
    }
}

pub fn action(recall: f64) -> FixedAction {
    FixedAction { recall }
}

pub fn artifact_key() -> ArtifactKey {
    ArtifactKey::ConfigCache([0xAA; 32])
}

pub fn prior_ptr() -> ArtifactPtr {
    ArtifactPtr::ConfigCacheKeyHash(PRIOR)
}

pub fn candidate_ptr() -> ArtifactPtr {
    ArtifactPtr::ConfigCacheKeyHash(CANDIDATE)
}

fn tripwires(root: &Path) -> TripwireRegistry {
    let mut registry = TripwireRegistry::load_from_vault(root).unwrap();
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
        seed: 399,
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

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn ascii_preview(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|byte| {
            if byte.is_ascii_graphic() || *byte == b':' {
                *byte as char
            } else {
                '.'
            }
        })
        .collect()
}

struct TestRoot(PathBuf);

impl TestRoot {
    fn new(label: &str) -> Self {
        let path = env::temp_dir().join(format!(
            "calyx-substrate-{label}-{}-{}",
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
