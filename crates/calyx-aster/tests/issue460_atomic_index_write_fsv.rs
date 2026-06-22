//! Ignored manual FSV for PH54 T04 atomic data+index write maintenance.
//!
//! Trigger with:
//! `CALYX_ISSUE460_FSV_ROOT=/var/lib/calyx/data/fsv-issue460-atomic-index-<stamp> \
//! cargo test -p calyx-aster --test issue460_atomic_index_write_fsv -- --ignored --nocapture`

use std::fs;
use std::path::{Path, PathBuf};

use calyx_aster::cf::ColumnFamily;
use calyx_aster::collection::{
    Collection, CollectionMode, DedupPolicy, FieldDef, FieldType, IsolationLevel, RetentionPolicy,
    Schema, SecondaryIndexKind, SecondaryIndexSpec, TemporalPolicy, TenantId, TxnPolicy,
    create_collection,
};
use calyx_aster::index::btree::btree_point;
use calyx_aster::index::{BtreeIndex, IndexId, IndexKind, IndexSpec, SecondaryIndex};
use calyx_aster::layers::kv::kv_key;
use calyx_aster::layers::relational::{self, collection_id};
use calyx_aster::layers::{KvLayer, RecordKey, RecordValue, RelationalLayer, Row};
use calyx_aster::vault::encode::decode_write_batch;
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_aster::wal::replay_dir;
use calyx_core::{Clock, VaultId};
use serde_json::{Value, json};

mod fsv_support;
use fsv_support::collect_physical_file_states;

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap()
}

fn index(kind: SecondaryIndexKind, name: &str, field: &str) -> SecondaryIndexSpec {
    SecondaryIndexSpec {
        name: name.to_string(),
        kind,
        fields: vec![field.to_string()],
    }
}

fn orders(name: &str, indexes: Vec<SecondaryIndexSpec>) -> Collection {
    Collection {
        name: name.to_string(),
        mode: CollectionMode::Records,
        schema: Some(Schema::SchemaFull(vec![
            FieldDef::new("item", FieldType::Text, false),
            FieldDef::new("qty", FieldType::I64, false),
        ])),
        panel: None,
        indexes,
        dedup: DedupPolicy::Off,
        temporal: TemporalPolicy::default(),
        retention: RetentionPolicy::Forever,
        txn_policy: TxnPolicy {
            isolation: IsolationLevel::ReadCommitted,
            cost_cap_ms: None,
        },
        tenant: TenantId::default(),
    }
}

fn kv_collection(name: &str, indexes: Vec<SecondaryIndexSpec>) -> Collection {
    Collection {
        name: name.to_string(),
        mode: CollectionMode::KV,
        schema: None,
        panel: None,
        indexes,
        dedup: DedupPolicy::Off,
        temporal: TemporalPolicy::default(),
        retention: RetentionPolicy::Forever,
        txn_policy: TxnPolicy::default(),
        tenant: TenantId::default(),
    }
}

fn qty_spec(id: u32) -> IndexSpec {
    IndexSpec::new(
        IndexId::new(id),
        "qty_idx",
        IndexKind::Btree,
        "qty",
        FieldType::I64,
    )
}

fn item_spec(id: u32) -> IndexSpec {
    IndexSpec::new(
        IndexId::new(id),
        "item_idx",
        IndexKind::Btree,
        "item",
        FieldType::Text,
    )
}

fn order_row(item: &str, qty: i64) -> Row {
    Row::new([
        ("item", RecordValue::Text(item.to_string())),
        ("qty", RecordValue::I64(qty)),
    ])
}

#[test]
#[ignore = "manual FSV writes atomic data/index evidence bytes"]
fn issue460_atomic_index_write_fsv_manual() {
    let root = PathBuf::from(
        std::env::var_os("CALYX_ISSUE460_FSV_ROOT")
            .expect("CALYX_ISSUE460_FSV_ROOT must point at a manual evidence root"),
    );
    fs::remove_dir_all(&root).ok();
    fs::create_dir_all(&root).unwrap();
    let vault_dir = root.join("vault");
    let vault = AsterVault::new_durable(
        &vault_dir,
        vault_id(),
        b"issue460-atomic-index-fsv".to_vec(),
        VaultOptions::default(),
    )
    .unwrap();
    let layer = RelationalLayer::new(&vault);
    let col = orders(
        "atomic_orders",
        vec![index(SecondaryIndexKind::Btree, "qty_idx", "qty")],
    );
    create_collection(&vault, col.clone()).unwrap();
    let pk = RecordKey::from_u64(7);
    let spec = qty_spec(1);
    let data_key = relational::record_key(&col, &pk).unwrap();
    let index_key = BtreeIndex::new(collection_id(&col), spec.clone())
        .encode_index_key(&RecordValue::I64(42), &pk)
        .unwrap();

    let before_seq = vault.latest_seq();
    let before = read_pair(&vault, before_seq, &data_key, &index_key);
    assert_eq!(before["data_present"], json!(false));
    assert_eq!(before["index_present"], json!(false));

    let put_seq = layer.put_record(&col, &pk, &order_row("bolt", 42)).unwrap();
    let after = read_pair(&vault, put_seq, &data_key, &index_key);
    assert_eq!(put_seq, before_seq + 1);
    assert_eq!(after["data_present"], json!(true));
    assert_eq!(after["index_present"], json!(true));
    assert_eq!(
        btree_point(&vault, &col, &spec, &RecordValue::I64(42)).unwrap(),
        vec![pk.clone()]
    );

    let no_index = edge_no_index(&vault, &layer);
    let missing = edge_missing_field(&vault, &layer);
    let update = edge_update_tombstone(&vault, &layer, &col, &spec, &pk);
    let two = edge_two_indexes(&vault, &layer);
    let kv_max_ns = edge_kv_max_namespace(&vault);

    vault.flush().unwrap();
    drop(vault);
    let reopened = AsterVault::open(
        &vault_dir,
        vault_id(),
        b"issue460-atomic-index-fsv".to_vec(),
        VaultOptions::default(),
    )
    .unwrap();
    let reopened_at_put_seq = read_pair(&reopened, put_seq, &data_key, &index_key);
    assert_eq!(reopened_at_put_seq["data_present"], json!(true));
    assert_eq!(reopened_at_put_seq["index_present"], json!(true));
    let reopened_latest_after_edges =
        read_pair(&reopened, reopened.latest_seq(), &data_key, &index_key);

    let wal_batches = wal_batches(&vault_dir.join("wal"));
    let put_batch = wal_batches
        .iter()
        .find(|batch| batch["seq"] == json!(put_seq))
        .cloned()
        .expect("put seq WAL batch");
    assert!(
        put_batch["cfs"]
            .as_array()
            .unwrap()
            .contains(&json!("relational"))
    );
    assert!(
        put_batch["cfs"]
            .as_array()
            .unwrap()
            .contains(&json!("index_btree"))
    );

    let readback = json!({
        "issue": 460,
        "trigger": "RelationalLayer::put_record(pk=7, qty=42)",
        "expected": {
            "one_seq": put_seq,
            "data_and_index_absent_before": true,
            "data_and_index_present_after": true,
            "btree_point_qty_42": [7],
        },
        "source_of_truth": {
            "vault": vault_dir.display().to_string(),
            "data_cf": "vault/cf/relational",
            "index_cf": "vault/cf/index_btree",
            "wal": "vault/wal",
        },
        "before": before,
        "after": after,
        "after_reopen_at_put_seq": reopened_at_put_seq,
        "after_reopen_latest_after_edges": reopened_latest_after_edges,
        "put_wal_batch": put_batch,
        "edges": {
            "no_index": no_index,
            "missing_indexed_field": missing,
            "update_tombstone": update,
            "two_indexes": two,
            "kv_max_namespace": kv_max_ns,
        },
        "final_relational_rows": rows_json(&reopened, ColumnFamily::Relational),
        "final_index_btree_rows": rows_json(&reopened, ColumnFamily::IndexBtree),
        "wal_batches": wal_batches,
        "physical_files": physical_files(&vault_dir),
    });

    fs::write(
        root.join("issue460-atomic-index-write-fsv-artifact.json"),
        serde_json::to_vec_pretty(&readback).unwrap(),
    )
    .unwrap();
    println!("{}", serde_json::to_string_pretty(&readback).unwrap());
}

fn edge_no_index<C: Clock>(vault: &AsterVault<C>, layer: &RelationalLayer<C>) -> Value {
    let col = orders("no_index_orders", Vec::new());
    create_collection(vault, col.clone()).unwrap();
    let before = vault
        .scan_cf_at(vault.latest_seq(), ColumnFamily::IndexBtree)
        .unwrap()
        .len();
    let seq = layer
        .put_record(&col, &RecordKey::from_u64(1), &order_row("plain", 1))
        .unwrap();
    let after = vault
        .scan_cf_at(seq, ColumnFamily::IndexBtree)
        .unwrap()
        .len();
    assert_eq!(before, after);
    json!({"before_index_rows": before, "after_index_rows": after, "seq": seq})
}

fn edge_missing_field<C: Clock>(vault: &AsterVault<C>, layer: &RelationalLayer<C>) -> Value {
    let col = orders(
        "missing_field_orders",
        vec![index(SecondaryIndexKind::Btree, "qty_idx", "qty")],
    );
    create_collection(vault, col.clone()).unwrap();
    let before_seq = vault.latest_seq();
    let before_rows = vault
        .scan_cf_at(before_seq, ColumnFamily::IndexBtree)
        .unwrap()
        .len();
    let err = layer
        .put_record(
            &col,
            &RecordKey::from_u64(2),
            &Row::new([("item", RecordValue::Text("bad".to_string()))]),
        )
        .unwrap_err();
    let after_rows = vault
        .scan_cf_at(vault.latest_seq(), ColumnFamily::IndexBtree)
        .unwrap()
        .len();
    assert_eq!(err.code, "CALYX_SCHEMA_VIOLATION");
    assert_eq!(before_seq, vault.latest_seq());
    assert_eq!(before_rows, after_rows);
    json!({"code": err.code, "before_seq": before_seq, "after_seq": vault.latest_seq(), "before_index_rows": before_rows, "after_index_rows": after_rows})
}

fn edge_update_tombstone<C: Clock>(
    vault: &AsterVault<C>,
    layer: &RelationalLayer<C>,
    col: &Collection,
    spec: &IndexSpec,
    pk: &RecordKey,
) -> Value {
    let seq = layer.put_record(col, pk, &order_row("bolt", 50)).unwrap();
    let old = btree_point(vault, col, spec, &RecordValue::I64(42)).unwrap();
    let new = btree_point(vault, col, spec, &RecordValue::I64(50)).unwrap();
    assert!(old.is_empty());
    assert_eq!(new, vec![pk.clone()]);
    json!({"seq": seq, "qty_42_pks": pk_nums(&old), "qty_50_pks": pk_nums(&new)})
}

fn edge_two_indexes<C: Clock>(vault: &AsterVault<C>, layer: &RelationalLayer<C>) -> Value {
    let col = orders(
        "two_index_orders",
        vec![
            index(SecondaryIndexKind::Btree, "qty_idx", "qty"),
            index(SecondaryIndexKind::Btree, "item_idx", "item"),
        ],
    );
    create_collection(vault, col.clone()).unwrap();
    let before = vault
        .scan_cf_at(vault.latest_seq(), ColumnFamily::IndexBtree)
        .unwrap()
        .len();
    let seq = layer
        .put_record(&col, &RecordKey::from_u64(8), &order_row("nut", 8))
        .unwrap();
    let qty = btree_point(vault, &col, &qty_spec(1), &RecordValue::I64(8)).unwrap();
    let item = btree_point(
        vault,
        &col,
        &item_spec(2),
        &RecordValue::Text("nut".to_string()),
    )
    .unwrap();
    let after = vault
        .scan_cf_at(seq, ColumnFamily::IndexBtree)
        .unwrap()
        .len();
    assert_eq!(after, before + 2);
    json!({"seq": seq, "before_index_rows": before, "after_index_rows": after, "qty_pks": pk_nums(&qty), "item_pks": pk_nums(&item)})
}

fn edge_kv_max_namespace<C: Clock>(vault: &AsterVault<C>) -> Value {
    let layer = KvLayer::new(vault);
    let col = kv_collection(
        "kv_max_namespace",
        vec![
            index(SecondaryIndexKind::Btree, "ns_idx", "ns"),
            index(SecondaryIndexKind::Btree, "key_idx", "key"),
        ],
    );
    create_collection(vault, col.clone()).unwrap();
    let before = vault
        .scan_cf_at(vault.latest_seq(), ColumnFamily::IndexBtree)
        .unwrap()
        .len();
    let key = b"max-ns";
    let value = b"value";
    let seq = layer.kv_set(&col, u64::MAX, key, value, None).unwrap();
    let got = layer.kv_get(&col, u64::MAX, key).unwrap();
    assert_eq!(got, Some(value.to_vec()));
    let ns_spec = IndexSpec::new(
        IndexId::new(1),
        "ns_idx",
        IndexKind::Btree,
        "ns",
        FieldType::U64,
    );
    let key_spec = IndexSpec::new(
        IndexId::new(2),
        "key_idx",
        IndexKind::Btree,
        "key",
        FieldType::Bytes,
    );
    let kv_pk = RecordKey::from_bytes(kv_key(&col, u64::MAX, key)).unwrap();
    let ns_index_key = BtreeIndex::new(collection_id(&col), ns_spec.clone())
        .encode_index_key(&RecordValue::U64(u64::MAX), &kv_pk)
        .unwrap();
    let key_index_key = BtreeIndex::new(collection_id(&col), key_spec.clone())
        .encode_index_key(&RecordValue::Bytes(key.to_vec()), &kv_pk)
        .unwrap();
    let ns_index_value = vault
        .read_cf_at(seq, ColumnFamily::IndexBtree, &ns_index_key)
        .unwrap();
    let key_index_value = vault
        .read_cf_at(seq, ColumnFamily::IndexBtree, &key_index_key)
        .unwrap();
    let ns_pks = btree_point(vault, &col, &ns_spec, &RecordValue::U64(u64::MAX)).unwrap();
    let key_pks = btree_point(vault, &col, &key_spec, &RecordValue::Bytes(key.to_vec())).unwrap();
    let after = vault
        .scan_cf_at(seq, ColumnFamily::IndexBtree)
        .unwrap()
        .len();
    assert_eq!(after, before + 2);
    assert_eq!(ns_index_value, Some(Vec::new()));
    assert_eq!(key_index_value, Some(Vec::new()));
    assert_eq!(ns_pks.len(), 1);
    assert_eq!(key_pks.len(), 1);
    assert_eq!(ns_pks[0], kv_pk);
    assert_eq!(key_pks[0], kv_pk);
    json!({
        "seq": seq,
        "before_index_rows": before,
        "after_index_rows": after,
        "kv_pk_hex": hex(kv_pk.as_bytes()),
        "ns_index_key_hex": hex(&ns_index_key),
        "key_index_key_hex": hex(&key_index_key),
        "value_hex": hex(&got.unwrap()),
        "ns_index_pks": pk_hexes(&ns_pks),
        "key_index_pks": pk_hexes(&key_pks),
    })
}

fn read_pair<C: Clock>(
    vault: &AsterVault<C>,
    seq: u64,
    data_key: &[u8],
    index_key: &[u8],
) -> Value {
    let data = vault
        .read_cf_at(seq, ColumnFamily::Relational, data_key)
        .unwrap();
    let index = vault
        .read_cf_at(seq, ColumnFamily::IndexBtree, index_key)
        .unwrap();
    json!({
        "seq": seq,
        "data_present": data.is_some(),
        "index_present": index.is_some(),
        "data_key_hex": hex(data_key),
        "index_key_hex": hex(index_key),
        "data_value_hex": data.as_ref().map(|bytes| hex(bytes)),
        "index_value_hex": index.as_ref().map(|bytes| hex(bytes)),
    })
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
                "offset": [record.start_offset, record.end_offset],
                "cfs": rows.iter().map(|row| row.cf.name()).collect::<Vec<_>>(),
                "rows": rows.iter().map(|row| json!({
                    "cf": row.cf.name(),
                    "key_hex": hex(&row.key),
                    "value_len": row.value.len(),
                    "value_hex_prefix": hex_prefix(&row.value, 64),
                })).collect::<Vec<_>>(),
            })
        })
        .collect()
}

fn rows_json<C: Clock>(vault: &AsterVault<C>, cf: ColumnFamily) -> Vec<Value> {
    vault
        .scan_cf_at(vault.latest_seq(), cf)
        .unwrap()
        .into_iter()
        .map(|(key, value)| json!({"key_hex": hex(&key), "value_len": value.len()}))
        .collect()
}

fn pk_nums(keys: &[RecordKey]) -> Vec<u64> {
    keys.iter()
        .map(|pk| u64::from_be_bytes(pk.as_bytes().try_into().unwrap()))
        .collect()
}

fn pk_hexes(keys: &[RecordKey]) -> Vec<String> {
    keys.iter().map(|pk| hex(pk.as_bytes())).collect()
}

fn physical_files(root: &Path) -> Vec<Value> {
    let mut files = Vec::new();
    collect_physical_file_states(root, &mut files);
    files.sort_by(|a, b| a["path"].as_str().cmp(&b["path"].as_str()));
    files
}

fn hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<Vec<_>>()
        .join("")
}

fn hex_prefix(bytes: &[u8], max: usize) -> String {
    hex(&bytes[..bytes.len().min(max)])
}
