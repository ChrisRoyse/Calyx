use std::fs;
use std::path::PathBuf;

use calyx_aster::cf::ColumnFamily;
use calyx_aster::collection::{
    Collection, CollectionMode, DedupPolicy, FieldDef, FieldType, IsolationLevel, RetentionPolicy,
    Schema, SecondaryIndexKind, SecondaryIndexSpec, TemporalPolicy, TenantId, TxnPolicy,
    create_collection,
};
use calyx_aster::index::btree::btree_range;
use calyx_aster::index::{
    BtreeIndex, IndexId, IndexKind, IndexSpec, InvertedIndex, SecondaryIndex,
};
use calyx_aster::layers::relational::{self, collection_id, encode_record_value};
use calyx_aster::layers::{RecordKey, RecordValue, RelationalLayer, Row};
use calyx_aster::mvcc::tombstone_value;
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::VaultId;
use serde_json::json;

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap()
}

fn index(field: &str) -> SecondaryIndexSpec {
    SecondaryIndexSpec {
        name: format!("{field}_idx"),
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
        tenant: TenantId(461),
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

fn item_inverted_spec() -> IndexSpec {
    IndexSpec::new(
        IndexId::new(1),
        "item_idx",
        IndexKind::Inverted,
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

fn vault() -> AsterVault {
    AsterVault::new(vault_id(), b"issue461-index-rebuild")
}

fn index_key(col: &Collection, spec: &IndexSpec, qty: i64, pk: &RecordKey) -> Vec<u8> {
    BtreeIndex::new(collection_id(col), spec.clone())
        .encode_index_key(&RecordValue::I64(qty), pk)
        .unwrap()
}

#[test]
fn verify_reports_healthy_for_five_records() {
    let vault = vault();
    let col = orders("healthy_orders", vec![index("qty")]);
    let spec = qty_spec();
    create_collection(&vault, col.clone()).unwrap();
    let layer = RelationalLayer::new(&vault);
    for qty in 1..=5 {
        layer
            .put_record(
                &col,
                &RecordKey::from_u64(qty as u64),
                &order_row("bolt", qty),
            )
            .unwrap();
    }

    let health = calyx_aster::index::index_verify(&vault, &col, &spec).unwrap();
    assert_eq!(health.missing, 0);
    assert_eq!(health.stale, 0);
    assert!(health.healthy);
}

#[test]
fn inverted_rebuild_repairs_missing_posting() {
    let vault = vault();
    let col = orders(
        "inverted_orders",
        vec![SecondaryIndexSpec {
            name: "item_idx".to_string(),
            kind: SecondaryIndexKind::Inverted,
            fields: vec!["item".to_string()],
        }],
    );
    let spec = item_inverted_spec();
    create_collection(&vault, col.clone()).unwrap();
    let layer = RelationalLayer::new(&vault);
    layer
        .put_record(&col, &RecordKey::from_u64(1), &order_row("alpha", 1))
        .unwrap();
    let pk = RecordKey::from_u64(2);
    layer.put_record(&col, &pk, &order_row("bravo", 2)).unwrap();
    let key = InvertedIndex::new(collection_id(&col), spec.clone())
        .encode_index_key(&RecordValue::Text("bravo".to_string()), &pk)
        .unwrap();
    vault
        .write_cf(ColumnFamily::IndexInverted, key, tombstone_value())
        .unwrap();

    let gap = calyx_aster::index::index_verify(&vault, &col, &spec).unwrap();
    assert_eq!(gap.missing, 1);
    assert_eq!(gap.stale, 0);
    let stats = calyx_aster::index::index_rebuild(&vault, &col, &spec, 1).unwrap();
    assert_eq!(stats.rows_scanned, 2);
    assert_eq!(stats.keys_added, 1);
    assert_eq!(stats.stale_removed, 0);
    assert!(
        calyx_aster::index::index_verify(&vault, &col, &spec)
            .unwrap()
            .healthy
    );
}

#[test]
fn rebuild_repairs_missing_stale_and_second_run_is_noop() {
    let vault = vault();
    let col = orders("repair_orders", vec![index("qty")]);
    let spec = qty_spec();
    create_collection(&vault, col.clone()).unwrap();
    let layer = RelationalLayer::new(&vault);
    for qty in 1..=5 {
        layer
            .put_record(
                &col,
                &RecordKey::from_u64(qty as u64),
                &order_row("nut", qty),
            )
            .unwrap();
    }
    for qty in [2_i64, 4] {
        let pk = RecordKey::from_u64(qty as u64);
        vault
            .write_cf(
                ColumnFamily::IndexBtree,
                index_key(&col, &spec, qty, &pk),
                tombstone_value(),
            )
            .unwrap();
    }

    let gap = calyx_aster::index::index_verify(&vault, &col, &spec).unwrap();
    assert_eq!(gap.missing, 2);
    let repaired = calyx_aster::index::index_rebuild(&vault, &col, &spec, 1).unwrap();
    assert_eq!(repaired.rows_scanned, 5);
    assert_eq!(repaired.keys_added, 2);
    assert_eq!(repaired.stale_removed, 0);
    assert!(
        calyx_aster::index::index_verify(&vault, &col, &spec)
            .unwrap()
            .healthy
    );

    let stale_pk = RecordKey::from_u64(5);
    vault
        .write_cf(
            ColumnFamily::Relational,
            relational::record_key(&col, &stale_pk).unwrap(),
            tombstone_value(),
        )
        .unwrap();
    let stale = calyx_aster::index::index_verify(&vault, &col, &spec).unwrap();
    assert_eq!(stale.stale, 1);
    let cleaned = calyx_aster::index::index_rebuild(&vault, &col, &spec, 2).unwrap();
    assert_eq!(cleaned.keys_added, 0);
    assert_eq!(cleaned.stale_removed, 1);
    assert!(
        calyx_aster::index::index_verify(&vault, &col, &spec)
            .unwrap()
            .healthy
    );

    let noop = calyx_aster::index::index_rebuild(&vault, &col, &spec, 2).unwrap();
    assert_eq!(noop.keys_added, 0);
    assert_eq!(noop.stale_removed, 0);
}

#[test]
fn edge_empty_collection_no_index_and_batch_limit() {
    let vault = vault();
    let spec = qty_spec();
    let empty = orders("empty_orders", vec![index("qty")]);
    create_collection(&vault, empty.clone()).unwrap();
    let stats = calyx_aster::index::index_rebuild(&vault, &empty, &spec, 0).unwrap();
    assert_eq!(stats.rows_scanned, 0);
    assert_eq!(stats.keys_added, 0);
    assert!(
        calyx_aster::index::index_verify(&vault, &empty, &spec)
            .unwrap()
            .healthy
    );

    let no_index = orders("no_index_orders", Vec::new());
    create_collection(&vault, no_index.clone()).unwrap();
    RelationalLayer::new(&vault)
        .put_record(&no_index, &RecordKey::from_u64(1), &order_row("plain", 7))
        .unwrap();
    assert_eq!(
        calyx_aster::index::index_rebuild(&vault, &no_index, &spec, 1).unwrap(),
        calyx_aster::index::RebuildStats::default()
    );
    let too_large = calyx_aster::index::index_rebuild(&vault, &empty, &spec, 10_001).unwrap_err();
    assert_eq!(too_large.code, "CALYX_INVALID_ARGUMENT");
}

#[test]
fn corrupt_data_row_fails_closed_with_snapshot_seq() {
    let vault = vault();
    let col = orders("corrupt_orders", vec![index("qty")]);
    let spec = qty_spec();
    create_collection(&vault, col.clone()).unwrap();
    let pk = RecordKey::from_u64(99);
    let key = relational::record_key(&col, &pk).unwrap();
    vault
        .write_cf(ColumnFamily::Relational, key.clone(), b"bad-row".to_vec())
        .unwrap();

    let error = calyx_aster::index::index_rebuild(&vault, &col, &spec, 1).unwrap_err();
    assert_eq!(error.code, "CALYX_ASTER_CORRUPT_SHARD");
    assert!(error.message.contains("snapshot_seq="));
    assert!(error.message.contains(&hex(&key)));
}

#[test]
fn changed_data_value_removes_old_index_key_as_stale() {
    let vault = vault();
    let col = orders("changed_value_orders", vec![index("qty")]);
    let spec = qty_spec();
    create_collection(&vault, col.clone()).unwrap();
    let pk = RecordKey::from_u64(1);
    let old_key = index_key(&col, &spec, 9, &pk);
    let new_row = order_row("washer", 11);
    vault
        .write_cf(
            ColumnFamily::Relational,
            relational::record_key(&col, &pk).unwrap(),
            encode_record_value(&new_row).unwrap(),
        )
        .unwrap();
    vault
        .write_cf(ColumnFamily::IndexBtree, old_key.clone(), Vec::new())
        .unwrap();

    let health = calyx_aster::index::index_verify(&vault, &col, &spec).unwrap();
    assert_eq!(health.missing, 1);
    assert_eq!(health.stale, 1);
    let stats = calyx_aster::index::index_rebuild(&vault, &col, &spec, 1).unwrap();
    assert_eq!(stats.keys_added, 1);
    assert_eq!(stats.stale_removed, 1);
    assert!(
        vault
            .read_cf_at(vault.latest_seq(), ColumnFamily::IndexBtree, &old_key)
            .unwrap()
            .is_none()
    );
}

#[test]
#[ignore = "manual FSV writes index rebuild evidence bytes"]
fn issue461_index_rebuild_fsv_manual() {
    let root = PathBuf::from(
        std::env::var_os("CALYX_ISSUE461_FSV_ROOT")
            .expect("CALYX_ISSUE461_FSV_ROOT must point at a fresh manual evidence root"),
    );
    if root.exists() {
        panic!("CALYX_ISSUE461_FSV_ROOT must be fresh: {}", root.display());
    }
    fs::create_dir_all(&root).unwrap();
    let vault_dir = root.join("vault");
    let vault = AsterVault::new_durable(
        &vault_dir,
        vault_id(),
        b"issue461-index-rebuild-fsv".to_vec(),
        VaultOptions::default(),
    )
    .unwrap();
    let col = orders("issue461_fsv_orders", vec![index("qty")]);
    let spec = qty_spec();
    create_collection(&vault, col.clone()).unwrap();
    let layer = RelationalLayer::new(&vault);
    for qty in 1..=5 {
        layer
            .put_record(
                &col,
                &RecordKey::from_u64(qty as u64),
                &order_row("fsv", qty),
            )
            .unwrap();
    }
    let before = rows_json(&vault, ColumnFamily::IndexBtree);
    let pk3 = RecordKey::from_u64(3);
    vault
        .write_cf(
            ColumnFamily::IndexBtree,
            index_key(&col, &spec, 3, &pk3),
            tombstone_value(),
        )
        .unwrap();
    let gap = calyx_aster::index::index_verify(&vault, &col, &spec).unwrap();
    let stats = calyx_aster::index::index_rebuild(&vault, &col, &spec, 1).unwrap();
    let after_health = calyx_aster::index::index_verify(&vault, &col, &spec).unwrap();
    let range = btree_range(
        &vault,
        &col,
        &spec,
        Some(&RecordValue::I64(1)),
        Some(&RecordValue::I64(10)),
        0,
    )
    .unwrap();
    let empty_col = orders("issue461_fsv_empty", vec![index("qty")]);
    create_collection(&vault, empty_col.clone()).unwrap();
    let empty_stats = calyx_aster::index::index_rebuild(&vault, &empty_col, &spec, 0).unwrap();
    let no_index = orders("issue461_fsv_no_index", Vec::new());
    create_collection(&vault, no_index.clone()).unwrap();
    let no_index_stats = calyx_aster::index::index_rebuild(&vault, &no_index, &spec, 1).unwrap();
    let corrupt_col = orders("issue461_fsv_corrupt", vec![index("qty")]);
    create_collection(&vault, corrupt_col.clone()).unwrap();
    let corrupt_key = relational::record_key(&corrupt_col, &RecordKey::from_u64(99)).unwrap();
    vault
        .write_cf(
            ColumnFamily::Relational,
            corrupt_key.clone(),
            b"bad-row".to_vec(),
        )
        .unwrap();
    let corrupt = calyx_aster::index::index_rebuild(&vault, &corrupt_col, &spec, 1).unwrap_err();
    let after = rows_json(&vault, ColumnFamily::IndexBtree);
    vault.flush().unwrap();
    let evidence = json!({
        "trigger": "delete one qty_idx index key for pk=3, verify, rebuild, verify, range-query",
        "expected": {"gap_missing": 1, "keys_added": 1, "healthy": true, "range_pks": [1, 2, 3, 4, 5]},
        "before_index_btree": before,
        "gap_health": gap,
        "rebuild_stats": stats,
        "after_health": after_health,
        "range_pks": pk_nums(&range),
        "edge_empty_stats": empty_stats,
        "edge_no_index_stats": no_index_stats,
        "edge_corrupt_error": {"code": corrupt.code, "message": corrupt.message},
        "after_index_btree": after,
        "vault_dir": vault_dir,
    });
    fs::write(
        root.join("issue461-index-rebuild-fsv.json"),
        serde_json::to_vec_pretty(&evidence).unwrap(),
    )
    .unwrap();
    println!("{}", serde_json::to_string_pretty(&evidence).unwrap());
    assert_eq!(gap.missing, 1);
    assert_eq!(stats.keys_added, 1);
    assert!(after_health.healthy);
    assert_eq!(pk_nums(&range), vec![1, 2, 3, 4, 5]);
    assert_eq!(empty_stats.rows_scanned, 0);
    assert_eq!(no_index_stats, calyx_aster::index::RebuildStats::default());
    assert_eq!(corrupt.code, "CALYX_ASTER_CORRUPT_SHARD");
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn rows_json(vault: &AsterVault, cf: ColumnFamily) -> Vec<serde_json::Value> {
    vault
        .scan_cf_at(vault.latest_seq(), cf)
        .unwrap()
        .into_iter()
        .map(|(key, value)| {
            json!({
                "key": hex(&key),
                "value": hex(&value),
            })
        })
        .collect()
}

fn pk_nums(keys: &[RecordKey]) -> Vec<u64> {
    keys.iter()
        .map(|pk| u64::from_be_bytes(pk.as_bytes().try_into().unwrap()))
        .collect()
}
