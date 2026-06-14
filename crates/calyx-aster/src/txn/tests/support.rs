use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use calyx_core::{
    CxFlags, CxId, FixedClock, InputRef, LedgerRef, Modality, SlotId, SlotVector, VaultId,
};
use serde_json::{Value, json};

use super::{IsolationLevel, TxnHandle, vault_id};
use crate::cf::{ColumnFamily, slot_key};
use crate::collection::{
    Collection, CollectionMode, DedupPolicy, FieldDef, FieldType, RetentionPolicy, Schema,
    TemporalPolicy, TenantId, TxnPolicy, create_collection,
};
use crate::layers::kv;
use crate::layers::relational::{RecordKey, RecordValue, Row, record_key};
use crate::vault::encode::decode_write_batch;
use crate::vault::{AsterVault, VaultOptions};
use crate::wal::replay_dir;

mod edges;

pub(super) struct Collections {
    pub orders: Collection,
    pub docs: Collection,
    pub cache: Collection,
    pub metrics: Collection,
    pub assets: Collection,
}

impl Collections {
    pub(super) fn create(vault: &AsterVault<FixedClock>, prefix: &str) -> Self {
        let cols = Self {
            orders: orders(&format!("{prefix}_orders")),
            docs: collection(&format!("{prefix}_docs"), CollectionMode::Documents),
            cache: collection(&format!("{prefix}_cache"), CollectionMode::KV),
            metrics: collection(&format!("{prefix}_metrics"), CollectionMode::TimeSeries),
            assets: collection(&format!("{prefix}_assets"), CollectionMode::Blob),
        };
        for col in [
            &cols.orders,
            &cols.docs,
            &cols.cache,
            &cols.metrics,
            &cols.assets,
        ] {
            create_collection(vault, col.clone()).unwrap();
        }
        cols
    }
}

pub(super) fn fsv_evidence(root: &Path) -> Value {
    let vault_dir = root.join("vault");
    let vault = durable_vault(&vault_dir);
    let cols = Collections::create(&vault, "fsv");
    let handle = TxnHandle::new(vault.vault_id());
    let pk = RecordKey::from_u64(463);
    let cx = constellation(vault.vault_id(), 463);
    let rel_key = record_key(&cols.orders, &pk).unwrap();
    let kv_key = kv::kv_key(&cols.cache, 7, b"fsv-session");
    let slot_key = slot_key(cx.cx_id);
    let before_seq = vault.latest_seq();
    let before = read_fsv_rows(&vault, before_seq, &rel_key, &kv_key, &slot_key);
    let expected_seq = before_seq + 1;
    let mut txn = handle
        .begin_on(
            &vault,
            IsolationLevel::Serializable,
            Some(500),
            Duration::from_millis(50),
        )
        .unwrap();
    txn.put_record(&vault, &cols.orders, &pk, &order_row("fsv", 463))
        .unwrap();
    txn.kv_set(&vault, &cols.cache, 7, b"fsv-session", b"active", None)
        .unwrap();
    txn.put_constellation(&vault, &cx).unwrap();
    let seq = txn.commit(&vault).unwrap();
    assert_eq!(seq, expected_seq);
    let after = read_fsv_rows(&vault, seq, &rel_key, &kv_key, &slot_key);
    vault.flush().unwrap();
    json!({
        "issue": 463,
        "source_of_truth": {
            "vault": vault_dir.display().to_string(),
            "relational_cf": vault_dir.join("cf/relational").display().to_string(),
            "kv_cf": vault_dir.join("cf/kv").display().to_string(),
            "slot_00_cf": vault_dir.join("cf/slot_00").display().to_string(),
            "wal": vault_dir.join("wal").display().to_string()
        },
        "synthetic_input": {
            "record_pk": 463,
            "record_item": "fsv",
            "record_qty": 463,
            "kv_namespace": 7,
            "kv_key": "fsv-session",
            "kv_value": "active",
            "slot_00_dim": 2
        },
        "hand_expected": {
            "commit_seq": expected_seq,
            "relational_present": true,
            "kv_present": true,
            "slot_00_present": true,
            "all_rows_share_commit_seq": true
        },
        "trigger": "CrossModelTxn put_record + kv_set + put_constellation then commit",
        "expected_seq": expected_seq,
        "before": before,
        "after": after,
        "edge_cases": edges::edge_evidence(root),
        "wal_batches": wal_batches(&vault_dir.join("wal")),
        "cf_files": physical_files(&vault_dir.join("cf"))
    })
}

fn read_fsv_rows(
    vault: &AsterVault<FixedClock>,
    seq: u64,
    rel_key: &[u8],
    kv_key: &[u8],
    slot_key: &[u8],
) -> Value {
    json!({
        "seq": seq,
        "relational_seq": vault.seq_for_key_at(seq, ColumnFamily::Relational, rel_key).unwrap(),
        "kv_seq": vault.seq_for_key_at(seq, ColumnFamily::Kv, kv_key).unwrap(),
        "slot_00_seq": vault.seq_for_key_at(seq, ColumnFamily::slot(SlotId::new(0)), slot_key).unwrap(),
        "relational_present": vault.read_cf_at(seq, ColumnFamily::Relational, rel_key).unwrap().is_some(),
        "kv_present": vault.read_cf_at(seq, ColumnFamily::Kv, kv_key).unwrap().is_some(),
        "slot_00_present": vault.read_cf_at(seq, ColumnFamily::slot(SlotId::new(0)), slot_key).unwrap().is_some()
    })
}

pub(super) fn write_fsv(root: &Path, evidence: &Value) {
    fs::write(
        root.join("issue463-cross-model-txn-readback.json"),
        serde_json::to_vec_pretty(evidence).unwrap(),
    )
    .unwrap();
    println!("{}", serde_json::to_string_pretty(evidence).unwrap());
    assert_eq!(
        evidence["after"]["relational_seq"],
        evidence["expected_seq"]
    );
    assert_eq!(evidence["after"]["kv_seq"], evidence["expected_seq"]);
    assert_eq!(evidence["after"]["slot_00_seq"], evidence["expected_seq"]);
    assert_eq!(
        evidence["edge_cases"]["cost_cap_exceeded"]["error_code"],
        "CALYX_TXN_COST_CAP"
    );
    assert_eq!(
        evidence["edge_cases"]["wal_submit_failure"]["error_code"],
        "CALYX_DISK_PRESSURE"
    );
}

pub(super) fn memory_vault() -> AsterVault<FixedClock> {
    AsterVault::with_clock(vault_id(), b"issue463", FixedClock::new(463_000))
}

pub(super) fn durable_vault(root: &Path) -> AsterVault<FixedClock> {
    fs::remove_dir_all(root).ok();
    AsterVault::new_durable_with_clock(
        root,
        vault_id(),
        b"issue463".to_vec(),
        VaultOptions::default(),
        FixedClock::new(463_000),
    )
    .unwrap()
}

pub(super) fn temp_root(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("calyx-{name}"));
    fs::remove_dir_all(&root).ok();
    fs::create_dir_all(&root).unwrap();
    root
}

fn orders(name: &str) -> Collection {
    let mut col = collection(name, CollectionMode::Records);
    col.schema = Some(Schema::SchemaFull(vec![
        FieldDef::new("item", FieldType::Text, false),
        FieldDef::new("qty", FieldType::I64, false),
    ]));
    col
}

fn collection(name: &str, mode: CollectionMode) -> Collection {
    Collection {
        name: name.to_string(),
        mode,
        schema: Some(Schema::SchemaLess),
        panel: None,
        indexes: Vec::new(),
        dedup: DedupPolicy::Off,
        temporal: TemporalPolicy::default(),
        retention: RetentionPolicy::Forever,
        txn_policy: TxnPolicy {
            isolation: IsolationLevel::Serializable,
            cost_cap_ms: Some(500),
        },
        tenant: TenantId(463),
    }
}

pub(super) fn order_row(item: &str, qty: i64) -> Row {
    Row::new([
        ("item", RecordValue::Text(item.to_string())),
        ("qty", RecordValue::I64(qty)),
    ])
}

pub(super) fn constellation(vault_id: VaultId, idx: u64) -> calyx_core::Constellation {
    let input = format!("issue463-input-{idx}");
    let mut hash = [0_u8; 32];
    hash.copy_from_slice(blake3::hash(input.as_bytes()).as_bytes());
    let mut slots = BTreeMap::new();
    slots.insert(
        SlotId::new(0),
        SlotVector::Dense {
            dim: 2,
            data: vec![idx as f32, 0.5],
        },
    );
    calyx_core::Constellation {
        cx_id: CxId::from_input(input.as_bytes(), 1, b"issue463"),
        vault_id,
        panel_version: 1,
        created_at: 463_000,
        input_ref: InputRef {
            hash,
            pointer: None,
            redacted: false,
        },
        modality: Modality::Structured,
        slots,
        scalars: BTreeMap::new(),
        metadata: BTreeMap::new(),
        anchors: Vec::new(),
        provenance: LedgerRef {
            seq: 0,
            hash: [0; 32],
        },
        flags: CxFlags {
            ungrounded: false,
            degraded: false,
            novel_region: false,
            redacted_input: false,
        },
    }
}

fn wal_batches(wal_dir: &Path) -> Vec<Value> {
    replay_dir(wal_dir)
        .unwrap()
        .records
        .into_iter()
        .map(|record| {
            let rows = decode_write_batch(&record.payload).unwrap();
            json!({
                "seq": record.seq,
                "cfs": rows.iter().map(|row| row.cf.name()).collect::<Vec<_>>()
            })
        })
        .collect()
}

fn physical_files(dir: &Path) -> Vec<Value> {
    let mut files = Vec::new();
    if dir.exists() {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                files.extend(physical_files(&path));
            } else {
                files.push(
                    json!({"path": path.display().to_string(), "bytes": fs::read(&path).unwrap().len()}),
                );
            }
        }
    }
    files.sort_by_key(|file| file["path"].as_str().unwrap_or_default().to_string());
    files
}
