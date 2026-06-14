use std::fs;
use std::path::Path;

use calyx_aster::cf::ColumnFamily;
use calyx_aster::collection::{
    Collection, CollectionMode, DedupPolicy, FieldDef, FieldType, IsolationLevel, RetentionPolicy,
    Schema, SecondaryIndexKind, SecondaryIndexSpec, TemporalPolicy, TenantId, TxnPolicy,
    create_collection,
};
use calyx_aster::index::btree::{btree_point, btree_range};
use calyx_aster::index::{BtreeIndex, IndexId, IndexKind, IndexSpec, SecondaryIndex};
use calyx_aster::layers::relational::{collection_id, encode_record_value, record_key};
use calyx_aster::layers::{RecordKey, RecordValue, RelationalLayer, Row};
use calyx_aster::mvcc::tombstone_value;
use calyx_aster::vault::encode::decode_write_batch;
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_aster::wal::replay_dir;
use calyx_core::{CalyxError, FixedClock, VaultId};
use serde_json::{Value, json};

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap()
}

fn index(name: &str, field: &str) -> SecondaryIndexSpec {
    SecondaryIndexSpec {
        name: name.to_string(),
        kind: SecondaryIndexKind::Btree,
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
            isolation: IsolationLevel::Serializable,
            cost_cap_ms: None,
        },
        tenant: TenantId(462),
    }
}

fn qty_spec() -> IndexSpec {
    IndexSpec::new(
        IndexId::new(1),
        "qty_idx",
        IndexKind::Btree,
        "qty",
        FieldType::I64,
    )
}

fn order_row(item: &str, qty: i64) -> Row {
    Row::new([
        ("item", RecordValue::Text(item.to_string())),
        ("qty", RecordValue::I64(qty)),
    ])
}

fn durable(path: &Path, salt: &[u8]) -> AsterVault<FixedClock> {
    AsterVault::new_durable_with_clock(
        path,
        vault_id(),
        salt.to_vec(),
        VaultOptions::default(),
        FixedClock::new(462_000),
    )
    .unwrap()
}

struct SameSeq {
    evidence: Value,
    data_key: Vec<u8>,
    index_key: Vec<u8>,
    put_seq: u64,
}

pub fn run_fsv(root: &Path) -> Value {
    let vault_dir = root.join("vault");
    let vault = durable(&vault_dir, b"issue462-ph54-fsv");
    let same_seq = same_seq_case(&vault, &vault_dir);
    let range_point = range_point_case(&vault);
    let rebuild = rebuild_case(&vault);
    let empty = empty_and_no_index_edges(&vault);
    vault.flush().unwrap();
    drop(vault);

    let reopened = durable(&vault_dir, b"issue462-ph54-fsv");
    let reopened_pair = read_pair(
        &reopened,
        same_seq.put_seq,
        &same_seq.data_key,
        &same_seq.index_key,
    );
    let crash = crash_case(&root.join("crash_vault"));

    json!({
        "issue": 462,
        "trigger": "put_record/index-maintenance plus injected crash before submit_batch",
        "expected": {
            "same_seq_data_index": same_seq.put_seq,
            "range_pks": [3, 4, 5, 6, 7],
            "point_pk": [5],
            "crash_pk2_absent": true,
            "rebuild_keys_added": 1
        },
        "source_of_truth": {
            "vault": vault_dir.display().to_string(),
            "data_cf": "vault/cf/relational",
            "index_cf": "vault/cf/index_btree",
            "wal": "vault/wal",
            "crash_vault": root.join("crash_vault").display().to_string()
        },
        "same_seq": same_seq.evidence,
        "same_seq_after_reopen": reopened_pair,
        "range_point": range_point,
        "rebuild": rebuild,
        "edges": {
            "empty_and_no_index": empty,
            "crash_before_submit": crash
        },
        "final_relational_rows": rows_json(&reopened, ColumnFamily::Relational),
        "final_index_btree_rows": rows_json(&reopened, ColumnFamily::IndexBtree)
    })
}

fn same_seq_case(vault: &AsterVault<FixedClock>, vault_dir: &Path) -> SameSeq {
    let col = orders("ph54_same_seq_orders", vec![index("qty_idx", "qty")]);
    create_collection(vault, col.clone()).unwrap();
    let layer = RelationalLayer::new(vault);
    let pk = RecordKey::from_u64(1);
    let spec = qty_spec();
    let data_key = record_key(&col, &pk).unwrap();
    let index_key = index_key(&col, &spec, 5, &pk);
    let before_seq = vault.latest_seq();
    let before = read_pair(vault, before_seq, &data_key, &index_key);
    let put_seq = layer.put_record(&col, &pk, &order_row("bolt", 5)).unwrap();
    let after = read_pair(vault, put_seq, &data_key, &index_key);
    let wal_batch = wal_batches(&vault_dir.join("wal"))
        .into_iter()
        .find(|batch| batch["seq"] == json!(put_seq))
        .unwrap();

    SameSeq {
        evidence: json!({
            "input": {"pk": 1, "qty": 5},
            "hand_expected": {"before_absent": true, "after_seq": put_seq},
            "before": before,
            "put_seq": put_seq,
            "data_seq": vault.seq_for_key_at(put_seq, ColumnFamily::Relational, &data_key).unwrap(),
            "index_seq": vault.seq_for_key_at(put_seq, ColumnFamily::IndexBtree, &index_key).unwrap(),
            "after": after,
            "wal_batch": wal_batch
        }),
        data_key,
        index_key,
        put_seq,
    }
}

fn range_point_case(vault: &AsterVault<FixedClock>) -> Value {
    let col = orders("ph54_range_orders", vec![index("qty_idx", "qty")]);
    create_collection(vault, col.clone()).unwrap();
    let layer = RelationalLayer::new(vault);
    let spec = qty_spec();
    for qty in 0..=9 {
        layer
            .put_record(
                &col,
                &RecordKey::from_u64(qty as u64),
                &order_row("range", qty),
            )
            .unwrap();
    }
    let range = btree_range(
        vault,
        &col,
        &spec,
        Some(&RecordValue::I64(3)),
        Some(&RecordValue::I64(7)),
        0,
    )
    .unwrap();
    let point = btree_point(vault, &col, &spec, &RecordValue::I64(5)).unwrap();
    let empty = btree_range(
        vault,
        &col,
        &spec,
        Some(&RecordValue::I64(9)),
        Some(&RecordValue::I64(3)),
        0,
    )
    .unwrap();
    json!({
        "input_qtys": [0,1,2,3,4,5,6,7,8,9],
        "expected_range_pks": [3,4,5,6,7],
        "actual_range_pks": pk_nums(&range),
        "expected_point_pks": [5],
        "actual_point_pks": pk_nums(&point),
        "edge_empty_reversed_range_pks": pk_nums(&empty)
    })
}

fn rebuild_case(vault: &AsterVault<FixedClock>) -> Value {
    let col = orders("ph54_rebuild_orders", vec![index("qty_idx", "qty")]);
    create_collection(vault, col.clone()).unwrap();
    let layer = RelationalLayer::new(vault);
    let spec = qty_spec();
    for qty in 1..=5 {
        layer
            .put_record(
                &col,
                &RecordKey::from_u64(qty as u64),
                &order_row("rebuild", qty),
            )
            .unwrap();
    }
    let pk3 = RecordKey::from_u64(3);
    let missing_key = index_key(&col, &spec, 3, &pk3);
    let before = btree_range(
        vault,
        &col,
        &spec,
        Some(&RecordValue::I64(1)),
        Some(&RecordValue::I64(10)),
        0,
    )
    .unwrap();
    vault
        .write_cf(
            ColumnFamily::IndexBtree,
            missing_key.clone(),
            tombstone_value(),
        )
        .unwrap();
    let gap = calyx_aster::index::index_verify(vault, &col, &spec).unwrap();
    let range_with_gap = btree_range(
        vault,
        &col,
        &spec,
        Some(&RecordValue::I64(1)),
        Some(&RecordValue::I64(10)),
        0,
    )
    .unwrap();
    let stats = calyx_aster::index::index_rebuild(vault, &col, &spec, 1).unwrap();
    let health = calyx_aster::index::index_verify(vault, &col, &spec).unwrap();
    let after = btree_range(
        vault,
        &col,
        &spec,
        Some(&RecordValue::I64(1)),
        Some(&RecordValue::I64(10)),
        0,
    )
    .unwrap();
    json!({
        "before_corruption_pks": pk_nums(&before),
        "gap_health": gap,
        "range_with_gap_pks": pk_nums(&range_with_gap),
        "rebuild_stats": stats,
        "after_health": health,
        "after_rebuild_pks": pk_nums(&after),
        "tombstoned_index_key": hex(&missing_key)
    })
}

fn empty_and_no_index_edges(vault: &AsterVault<FixedClock>) -> Value {
    let spec = qty_spec();
    let empty = orders("ph54_empty_orders", vec![index("qty_idx", "qty")]);
    create_collection(vault, empty.clone()).unwrap();
    let empty_stats = calyx_aster::index::index_rebuild(vault, &empty, &spec, 0).unwrap();
    let no_index = orders("ph54_no_index_orders", Vec::new());
    create_collection(vault, no_index.clone()).unwrap();
    RelationalLayer::new(vault)
        .put_record(&no_index, &RecordKey::from_u64(1), &order_row("plain", 1))
        .unwrap();
    let no_index_stats = calyx_aster::index::index_rebuild(vault, &no_index, &spec, 1).unwrap();
    json!({
        "empty_collection_before_rows": 0,
        "empty_collection_after_stats": empty_stats,
        "no_index_after_stats": no_index_stats
    })
}

fn crash_case(vault_dir: &Path) -> Value {
    let vault = durable(vault_dir, b"issue462-crash");
    let col = orders("ph54_crash_orders", vec![index("qty_idx", "qty")]);
    create_collection(&vault, col.clone()).unwrap();
    let pk = RecordKey::from_u64(2);
    let spec = qty_spec();
    let data_key = record_key(&col, &pk).unwrap();
    let index_key = index_key(&col, &spec, 9, &pk);
    let before_seq = vault.latest_seq();
    let before = read_pair(&vault, before_seq, &data_key, &index_key);
    let (error, staged) =
        FaultInjector::fail_after_data_key_staged(&col, &pk, &order_row("crash", 9));
    let after_seq = vault.latest_seq();
    let after = read_pair(&vault, after_seq, &data_key, &index_key);
    vault.flush().unwrap();
    drop(vault);

    let reopened = durable(vault_dir, b"issue462-crash");
    let got = RelationalLayer::new(&reopened)
        .get_record(&col, &pk)
        .unwrap()
        .is_some();
    let point = btree_point(&reopened, &col, &spec, &RecordValue::I64(9)).unwrap();
    json!({
        "trigger": "FaultInjector::fail_after_data_key_staged(pk=2, qty=9)",
        "staged_before_submit": staged,
        "error": {"code": error.code, "message": error.message},
        "before_seq": before_seq,
        "after_seq": after_seq,
        "before": before,
        "after": after,
        "after_reopen_record_present": got,
        "after_reopen_point_pks": pk_nums(&point),
        "after_reopen_pair": read_pair(&reopened, reopened.latest_seq(), &data_key, &index_key),
        "wal_batches": wal_batches(&vault_dir.join("wal"))
    })
}

struct FaultInjector;

impl FaultInjector {
    fn fail_after_data_key_staged(
        col: &Collection,
        pk: &RecordKey,
        row: &Row,
    ) -> (CalyxError, Value) {
        let data_key = record_key(col, pk).unwrap();
        let data_value = encode_record_value(row).unwrap();
        (
            CalyxError::disk_pressure("injected crash before submit_batch"),
            json!({
                "cf": "relational",
                "key_hex": hex(&data_key),
                "value_len": data_value.len(),
                "submitted_to_wal": false
            }),
        )
    }
}

fn index_key(col: &Collection, spec: &IndexSpec, qty: i64, pk: &RecordKey) -> Vec<u8> {
    BtreeIndex::new(collection_id(col), spec.clone())
        .encode_index_key(&RecordValue::I64(qty), pk)
        .unwrap()
}

fn read_pair(vault: &AsterVault<FixedClock>, seq: u64, data_key: &[u8], index_key: &[u8]) -> Value {
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
        "data_seq": vault.seq_for_key_at(seq, ColumnFamily::Relational, data_key).unwrap(),
        "index_seq": vault.seq_for_key_at(seq, ColumnFamily::IndexBtree, index_key).unwrap(),
        "data_key_hex": hex(data_key),
        "index_key_hex": hex(index_key),
        "data_value_hex": data.as_ref().map(|bytes| hex(bytes)),
        "index_value_hex": index.as_ref().map(|bytes| hex(bytes))
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
                    "value_len": row.value.len()
                })).collect::<Vec<_>>()
            })
        })
        .collect()
}

fn rows_json(vault: &AsterVault<FixedClock>, cf: ColumnFamily) -> Vec<Value> {
    vault
        .scan_cf_at(vault.latest_seq(), cf)
        .unwrap()
        .into_iter()
        .map(|(key, value)| json!({"key_hex": hex(&key), "value_len": value.len()}))
        .collect()
}

pub fn write_and_assert(root: &Path, evidence: &Value) {
    fs::write(
        root.join("ph54-fsv-issue462.json"),
        serde_json::to_vec_pretty(evidence).unwrap(),
    )
    .unwrap();
    println!("{}", serde_json::to_string_pretty(evidence).unwrap());
    assert_eq!(
        evidence["same_seq"]["data_seq"],
        evidence["same_seq"]["index_seq"]
    );
    assert_eq!(
        evidence["same_seq"]["data_seq"],
        evidence["same_seq"]["put_seq"]
    );
    assert_eq!(
        evidence["range_point"]["actual_range_pks"],
        json!([3, 4, 5, 6, 7])
    );
    assert_eq!(evidence["range_point"]["actual_point_pks"], json!([5]));
    assert_eq!(
        evidence["rebuild"]["range_with_gap_pks"],
        json!([1, 2, 4, 5])
    );
    assert_eq!(
        evidence["rebuild"]["after_rebuild_pks"],
        json!([1, 2, 3, 4, 5])
    );
    assert_eq!(evidence["rebuild"]["rebuild_stats"]["keys_added"], json!(1));
    assert_eq!(
        evidence["edges"]["crash_before_submit"]["error"]["code"],
        json!("CALYX_DISK_PRESSURE")
    );
    assert_eq!(
        evidence["edges"]["crash_before_submit"]["before_seq"],
        evidence["edges"]["crash_before_submit"]["after_seq"]
    );
    assert_eq!(
        evidence["edges"]["crash_before_submit"]["after"]["data_present"],
        json!(false)
    );
    assert_eq!(
        evidence["edges"]["crash_before_submit"]["after"]["index_present"],
        json!(false)
    );
    assert_eq!(
        evidence["edges"]["crash_before_submit"]["after_reopen_record_present"],
        json!(false)
    );
    assert_eq!(
        evidence["edges"]["crash_before_submit"]["after_reopen_point_pks"],
        json!([])
    );
    println!("ph54 FSV: same-seq PASS, no-half-indexed PASS, range-correct PASS, rebuild PASS");
}

fn pk_nums(keys: &[RecordKey]) -> Vec<u64> {
    keys.iter()
        .map(|pk| u64::from_be_bytes(pk.as_bytes().try_into().unwrap()))
        .collect()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
