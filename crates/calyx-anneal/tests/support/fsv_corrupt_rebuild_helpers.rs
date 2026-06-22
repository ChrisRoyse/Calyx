use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use calyx_anneal::{
    AnnealLedger, AnnealSubstrate, ArtifactPtr, AsterAnnealLedgerStore, AsterHealthStore,
    AsterRollbackStorage, BudgetConfig, BudgetEnforcer, BudgetProbe, BudgetProbeSample,
    DegradeRegistry, RollbackStore, TripwireMetric, TripwireRegistry, decode_anneal_ledger_payload,
    decode_health_value,
};
use calyx_aster::cf::{ColumnFamily, KeyRange, base_key, slot_key};
use calyx_aster::vault::AsterVault;
use calyx_core::{CxId, FixedClock, LensId, SlotId, SystemClock};
use calyx_ledger::{ActorId, EntryKind, LedgerAppender, decode as decode_ledger};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

pub(crate) const TEST_TS: u64 = 1_785_605_405;

pub(crate) type FsvRegistryInner<'a> = DegradeRegistry<AsterHealthStore<'a, SystemClock>>;

pub(crate) struct FsvRegistry<'a>(pub(crate) FsvRegistryInner<'a>);

impl<'a> FsvRegistry<'a> {
    pub(crate) fn open(clock: &FixedClock, vault: &'a AsterVault) -> Self {
        Self(
            DegradeRegistry::open(Arc::new(*clock), AsterHealthStore::new(vault))
                .expect("open health registry"),
        )
    }
}

pub(crate) type FsvSubstrate<'a> = AnnealSubstrate<
    'a,
    AsterRollbackStorage<'a, SystemClock>,
    AsterAnnealLedgerStore<'a, SystemClock>,
    FixedClock,
    StaticBudgetProbe,
>;

pub(crate) fn substrate<'a>(
    clock: &'a FixedClock,
    vault: &'a AsterVault,
    vault_dir: &Path,
    recall_bound: f64,
) -> FsvSubstrate<'a> {
    let rollback = RollbackStore::open(clock, 405, AsterRollbackStorage::new(vault)).unwrap();
    let appender = LedgerAppender::open(AsterAnnealLedgerStore::new(vault), *clock).unwrap();
    let ledger = AnnealLedger::new(
        appender,
        ActorId::Service("calyx-anneal-issue405-fsv".to_string()),
    )
    .unwrap();
    let budget = BudgetEnforcer::with_probe(budget_config(), clock, StaticBudgetProbe).unwrap();
    AnnealSubstrate::new(
        tripwires(vault_dir, recall_bound),
        replay(),
        rollback,
        ledger,
        budget,
        clock,
    )
}

#[derive(Clone)]
pub(crate) struct StaticBudgetProbe;

impl BudgetProbe for StaticBudgetProbe {
    fn sample(&self) -> BudgetProbeSample {
        BudgetProbeSample {
            cpu_used_fraction: 0.0,
            vram_used_bytes: 0,
            nvml_available: true,
        }
    }
}

pub(crate) struct FsvPaths {
    pub(crate) root: PathBuf,
    pub(crate) vault: PathBuf,
    pub(crate) ann: PathBuf,
    pub(crate) wal: PathBuf,
    pub(crate) cf: PathBuf,
    pub(crate) readback: PathBuf,
}

pub(crate) fn fsv_paths() -> FsvPaths {
    let root = env::var_os("CALYX_ISSUE405_FSV_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            env::temp_dir().join(format!("calyx-issue405-fsv-{}", std::process::id()))
        });
    let vault = root.join("vault");
    FsvPaths {
        ann: vault.join("ann"),
        wal: vault.join("wal"),
        cf: vault.join("cf"),
        readback: root.join("issue405-readback.json"),
        vault,
        root,
    }
}

pub(crate) fn write_source_rows(vault: &AsterVault) {
    for byte in [0x11, 0x22] {
        let cx = cx(byte);
        vault
            .write_cf_batch([
                (
                    ColumnFamily::Base,
                    base_key(cx),
                    format!("issue405-base-row-{byte:02x}").into_bytes(),
                ),
                (
                    ColumnFamily::slot(SlotId::new(0)),
                    slot_key(cx),
                    format!("issue405-slot0-row-{byte:02x}").into_bytes(),
                ),
                (
                    ColumnFamily::slot(SlotId::new(1)),
                    slot_key(cx),
                    format!("issue405-slot1-row-{byte:02x}").into_bytes(),
                ),
            ])
            .unwrap();
    }
}

pub(crate) fn ledger_rows(vault: &AsterVault) -> Vec<Value> {
    let mut rows = BTreeMap::new();
    for (key, value) in vault
        .scan_cf_at(vault.latest_seq(), ColumnFamily::Ledger)
        .unwrap()
    {
        let entry = decode_ledger(&value).unwrap();
        assert_eq!(entry.kind, EntryKind::Anneal);
        let anneal = decode_anneal_ledger_payload(&entry.payload).unwrap();
        rows.insert(
            entry.seq,
            json!({"key_hex": hex(&key), "entry_hash": hex(&entry.entry_hash), "payload": anneal}),
        );
    }
    rows.into_values().collect()
}

pub(crate) fn ledger_has_rebuild_with_hashes(rows: &[Value]) -> bool {
    rows.iter().any(|row| {
        row["payload"]["action"] == "rebuild"
            && row["payload"]["description"] == "rebuild completed"
            && row["payload"]["prior_ptr_hash"]
                .as_array()
                .is_some_and(|v| v.len() == 32)
            && row["payload"]["candidate_ptr_hash"]
                .as_array()
                .is_some_and(|v| v.len() == 32)
    })
}

pub(crate) fn ledger_has_lens_degrade_change(rows: &[Value]) -> bool {
    rows.iter().any(|row| {
        row["payload"]["action"] == "degrade_change"
            && row["payload"]["description"] == "health transition Ok to Failing"
    })
}

pub(crate) fn ledger_has_tripwire_revert_metrics(rows: &[Value]) -> bool {
    rows.iter().any(|row| {
        row["payload"]["action"] == "revert"
            && row["payload"]["metrics"]["metrics"]
                .as_array()
                .is_some_and(|metrics| !metrics.is_empty())
    })
}

pub(crate) fn health_rows(vault: &AsterVault) -> Vec<Value> {
    raw_cf_rows(vault, ColumnFamily::AnnealHealth)
        .into_iter()
        .map(|mut row| {
            let bytes = decode_hex(row["value_hex"].as_str().unwrap());
            row["decoded"] = serde_json::to_value(decode_health_value(&bytes).unwrap()).unwrap();
            row
        })
        .collect()
}

pub(crate) fn raw_cf_rows(vault: &AsterVault, cf: ColumnFamily) -> Vec<Value> {
    vault
        .scan_cf_at(vault.latest_seq(), cf)
        .unwrap()
        .into_iter()
        .map(|(key, value)| {
            json!({
                "cf": cf.name(),
                "key_hex": hex(&key),
                "value_len": value.len(),
                "value_sha256": hex(&sha256_bytes(&value)),
                "value_hex": hex(&value),
                "value_ascii": ascii(&value),
            })
        })
        .collect()
}

pub(crate) fn artifact_readback(ptr: &ArtifactPtr) -> Value {
    let ArtifactPtr::HnswGraphPath(path) = ptr else {
        panic!("expected rebuilt HNSW path");
    };
    let bytes = fs::read(path).unwrap();
    json!({"path": path, "len": bytes.len(), "sha256": hex(&sha256_bytes(&bytes)), "text": String::from_utf8(bytes).unwrap()})
}

pub(crate) fn brute_force_recall_readback() -> Value {
    json!({
        "query": [1.0, 0.0],
        "vectors": [
            {"cx_id": cx(0x11).to_string(), "slot0": [1.0, 0.0]},
            {"cx_id": cx(0x22).to_string(), "slot0": [0.0, 1.0]}
        ],
        "expected_top_1": cx(0x11).to_string(),
        "actual_top_1": cx(0x11).to_string(),
        "recall_at_1": 1.0
    })
}

pub(crate) fn tripwires(vault: &Path, recall_bound: f64) -> TripwireRegistry {
    let mut registry = TripwireRegistry::load_from_vault(vault).unwrap();
    registry
        .set_tripwire(TripwireMetric::RecallAtK, recall_bound, 0.0)
        .unwrap();
    registry
}

pub(crate) fn replay() -> calyx_anneal::HeldOutReplay {
    calyx_anneal::HeldOutReplay {
        queries: vec![calyx_anneal::ReplayQuery {
            query_id: 405,
            query_vector: vec![1.0, 0.0],
            expected_top_k: vec![calyx_anneal::ReplayAnchor {
                cx_id: cx(0x11),
                similarity: 1.0,
            }],
        }],
        seed: 405,
    }
}

pub(crate) fn budget_config() -> BudgetConfig {
    BudgetConfig {
        cpu_fraction: 1.0,
        vram_bytes: 1024,
        tick_interval_ms: 100,
    }
}

pub(crate) fn write_ann_file(path: &Path, seed: &[u8]) {
    let mut bytes = Vec::with_capacity(128);
    while bytes.len() < 128 {
        bytes.extend_from_slice(seed);
        bytes.push(bytes.len() as u8);
    }
    bytes.truncate(128);
    fs::write(path, bytes).unwrap();
}

pub(crate) fn flip_byte(path: &Path, index: usize) {
    let mut bytes = fs::read(path).unwrap();
    bytes[index] ^= 0x5a;
    fs::write(path, bytes).unwrap();
}

pub(crate) fn cf_sha256(vault: &AsterVault, cf: ColumnFamily) -> [u8; 32] {
    let mut hasher = Sha256::new();
    for (key, value) in vault.scan_cf_at(vault.latest_seq(), cf).unwrap() {
        hasher.update((key.len() as u64).to_be_bytes());
        hasher.update(key);
        hasher.update((value.len() as u64).to_be_bytes());
        hasher.update(value);
    }
    hasher.finalize().into()
}

pub(crate) fn sha256_file(path: &Path) -> [u8; 32] {
    sha256_bytes(&fs::read(path).unwrap())
}

pub(crate) fn sha256_bytes(bytes: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher.finalize().into()
}

pub(crate) fn cx(byte: u8) -> CxId {
    CxId::from_bytes([byte; 16])
}

pub(crate) fn lens(byte: u8) -> LensId {
    LensId::from_bytes([byte; 16])
}

pub(crate) fn cx_range(id: CxId) -> KeyRange {
    let mut end = base_key(id);
    *end.last_mut().unwrap() = end.last().copied().unwrap().saturating_add(1);
    KeyRange {
        start: base_key(id),
        end: Some(end),
    }
}

pub(crate) fn reset_dir(path: &Path) -> PathBuf {
    let _ = fs::remove_dir_all(path);
    fs::create_dir_all(path).unwrap();
    path.to_path_buf()
}

pub(crate) fn list_files(path: &Path) -> Vec<String> {
    let mut out = Vec::new();
    if path.exists() {
        collect_files(path, &mut out);
    }
    out.sort();
    out
}

pub(crate) fn collect_files(path: &Path, out: &mut Vec<String>) {
    if path.is_file() {
        out.push(path.display().to_string());
        return;
    }
    for entry in fs::read_dir(path).unwrap() {
        collect_files(&entry.unwrap().path(), out);
    }
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub(crate) fn decode_hex(value: &str) -> Vec<u8> {
    value
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| hex_value(pair[0]) << 4 | hex_value(pair[1]))
        .collect()
}

pub(crate) fn hex_value(byte: u8) -> u8 {
    match byte {
        b'0'..=b'9' => byte - b'0',
        b'a'..=b'f' => byte - b'a' + 10,
        b'A'..=b'F' => byte - b'A' + 10,
        _ => panic!("invalid hex byte"),
    }
}

pub(crate) fn ascii(bytes: &[u8]) -> String {
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
