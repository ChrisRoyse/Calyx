use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use calyx_core::{FixedClock, VaultId};
use proptest::prelude::*;

use super::*;
use crate::collection::{
    Collection, CollectionMode, DedupPolicy, FieldDef, IsolationLevel, RetentionPolicy,
    SecondaryIndexSpec, TemporalPolicy, TenantId, TxnPolicy, create_collection,
};
use crate::index::btree::{btree_point, btree_range};
use crate::index::inverted::{inverted_match, inverted_stats};
use crate::layers::relational::{collection_id, record_key};
use crate::layers::{RecordKey, RecordValue, RelationalLayer, Row};
use crate::vault::{AsterVault, VaultOptions};

static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap()
}

fn btree(name: &str, field: &str) -> SecondaryIndexSpec {
    SecondaryIndexSpec {
        name: name.to_string(),
        kind: SecondaryIndexKind::Btree,
        fields: vec![field.to_string()],
    }
}

fn inverted(name: &str, field: &str) -> SecondaryIndexSpec {
    SecondaryIndexSpec {
        name: name.to_string(),
        kind: SecondaryIndexKind::Inverted,
        fields: vec![field.to_string()],
    }
}

fn orders(indexes: Vec<SecondaryIndexSpec>) -> Collection {
    Collection {
        name: "orders".to_string(),
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

fn texts(indexes: Vec<SecondaryIndexSpec>) -> Collection {
    Collection {
        name: "texts".to_string(),
        mode: CollectionMode::Records,
        schema: Some(Schema::SchemaFull(vec![FieldDef::new(
            "body",
            FieldType::Text,
            false,
        )])),
        panel: None,
        indexes,
        dedup: DedupPolicy::Off,
        temporal: TemporalPolicy::default(),
        retention: RetentionPolicy::Forever,
        txn_policy: TxnPolicy::default(),
        tenant: TenantId::default(),
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

fn item_spec() -> IndexSpec {
    IndexSpec::new(
        IndexId::new(2),
        "item_idx",
        IndexKind::Btree,
        "item",
        FieldType::Text,
    )
}

fn body_spec() -> IndexSpec {
    IndexSpec::new(
        IndexId::new(1),
        "body_idx",
        IndexKind::Inverted,
        "body",
        FieldType::Text,
    )
}

fn row(item: &str, qty: i64) -> Row {
    Row::new([
        ("item", RecordValue::Text(item.to_string())),
        ("qty", RecordValue::I64(qty)),
    ])
}

#[test]
fn put_record_stages_data_and_btree_index_at_one_seq() {
    let vault = AsterVault::with_clock(vault_id(), b"salt", FixedClock::new(10));
    let layer = RelationalLayer::new(&vault);
    let col = orders(vec![btree("qty_idx", "qty")]);
    let pk = RecordKey::from_u64(7);
    let row = row("bolt", 42);
    let spec = qty_spec();
    let idx = BtreeIndex::new(collection_id(&col), spec.clone());
    let data_key = record_key(&col, &pk).unwrap();
    let index_key = idx.encode_index_key(&RecordValue::I64(42), &pk).unwrap();

    let before = vault.latest_seq();
    let seq = layer.put_record(&col, &pk, &row).unwrap();
    assert_eq!(seq, before + 1);
    assert_eq!(
        vault
            .read_cf_at(before, ColumnFamily::Relational, &data_key)
            .unwrap(),
        None
    );
    assert_eq!(
        vault
            .read_cf_at(before, ColumnFamily::IndexBtree, &index_key)
            .unwrap(),
        None
    );
    assert!(
        vault
            .read_cf_at(seq, ColumnFamily::Relational, &data_key)
            .unwrap()
            .is_some()
    );
    assert_eq!(
        vault
            .read_cf_at(seq, ColumnFamily::IndexBtree, &index_key)
            .unwrap(),
        Some(Vec::new())
    );
    assert_eq!(
        btree_point(&vault, &col, &spec, &RecordValue::I64(42)).unwrap(),
        vec![pk]
    );
}

#[test]
fn update_tombstones_old_index_key_and_keeps_new_key_visible() {
    let vault = AsterVault::with_clock(vault_id(), b"salt", FixedClock::new(10));
    let layer = RelationalLayer::new(&vault);
    let col = orders(vec![btree("qty_idx", "qty")]);
    let spec = qty_spec();
    let pk = RecordKey::from_u64(9);

    layer.put_record(&col, &pk, &row("bolt", 42)).unwrap();
    layer.put_record(&col, &pk, &row("bolt", 50)).unwrap();

    assert!(
        btree_point(&vault, &col, &spec, &RecordValue::I64(42))
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        btree_point(&vault, &col, &spec, &RecordValue::I64(50)).unwrap(),
        vec![pk]
    );
    assert_eq!(
        vault
            .scan_cf_at(vault.latest_seq(), ColumnFamily::IndexBtree)
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn no_indexes_noop_missing_field_fails_and_two_indexes_share_batch() {
    let vault = AsterVault::with_clock(vault_id(), b"salt", FixedClock::new(10));
    let layer = RelationalLayer::new(&vault);
    let no_index = orders(Vec::new());
    layer
        .put_record(&no_index, &RecordKey::from_u64(1), &row("bolt", 1))
        .unwrap();
    assert!(
        vault
            .scan_cf_at(vault.latest_seq(), ColumnFamily::IndexBtree)
            .unwrap()
            .is_empty()
    );

    let indexed = orders(vec![btree("qty_idx", "qty"), btree("item_idx", "item")]);
    let before = vault.latest_seq();
    let seq = layer
        .put_record(&indexed, &RecordKey::from_u64(2), &row("nut", 2))
        .unwrap();
    assert_eq!(seq, before + 1);
    assert_eq!(
        vault
            .scan_cf_at(seq, ColumnFamily::IndexBtree)
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        btree_point(&vault, &indexed, &qty_spec(), &RecordValue::I64(2)).unwrap(),
        vec![RecordKey::from_u64(2)]
    );
    assert_eq!(
        btree_point(
            &vault,
            &indexed,
            &item_spec(),
            &RecordValue::Text("nut".to_string()),
        )
        .unwrap(),
        vec![RecordKey::from_u64(2)]
    );

    let bad = Row::new([("item", RecordValue::Text("bolt".to_string()))]);
    let before_bad = vault.latest_seq();
    let error = layer
        .put_record(&indexed, &RecordKey::from_u64(3), &bad)
        .unwrap_err();
    assert_eq!(error.code, CALYX_SCHEMA_VIOLATION);
    assert_eq!(vault.latest_seq(), before_bad);
}

#[test]
fn inverted_index_rows_are_staged_with_relational_put() {
    let vault = AsterVault::with_clock(vault_id(), b"salt", FixedClock::new(10));
    let layer = RelationalLayer::new(&vault);
    let col = texts(vec![inverted("body_idx", "body")]);
    let spec = body_spec();
    let pk = RecordKey::from_u64(1);

    let before = vault.latest_seq();
    let seq = layer
        .put_record(
            &col,
            &pk,
            &Row::new([("body", RecordValue::Text("quick fox quick".to_string()))]),
        )
        .unwrap();

    assert_eq!(seq, before + 1);
    assert_eq!(
        inverted_match(&vault, &col, &spec, "quick").unwrap(),
        vec![(
            pk.clone(),
            inverted_match(&vault, &col, &spec, "quick").unwrap()[0].1
        )]
    );
    assert_eq!(inverted_stats(&vault, &col, &spec).unwrap().doc_count, 1);
    assert_eq!(
        vault
            .scan_cf_at(seq, ColumnFamily::IndexInverted)
            .unwrap()
            .len(),
        3
    );
}

#[test]
fn wal_failure_leaves_data_and_index_absent() {
    let dir = temp_dir("index-maintenance-wal-fail");
    let vault =
        AsterVault::new_durable(&dir, vault_id(), b"salt".to_vec(), VaultOptions::default())
            .unwrap();
    let layer = RelationalLayer::new(&vault);
    let col = orders(vec![btree("qty_idx", "qty")]);
    create_collection(&vault, col.clone()).unwrap();
    let pk = RecordKey::from_u64(7);
    let data_key = record_key(&col, &pk).unwrap();
    let index_key = BtreeIndex::new(collection_id(&col), qty_spec())
        .encode_index_key(&RecordValue::I64(7), &pk)
        .unwrap();

    vault.fail_next_wal_append_for_test();
    let error = layer.put_record(&col, &pk, &row("bolt", 7)).unwrap_err();
    assert_eq!(error.code, "CALYX_DISK_PRESSURE");
    assert_eq!(
        vault
            .read_cf_at(vault.latest_seq(), ColumnFamily::Relational, &data_key)
            .unwrap(),
        None
    );
    assert_eq!(
        vault
            .read_cf_at(vault.latest_seq(), ColumnFamily::IndexBtree, &index_key)
            .unwrap(),
        None
    );
    fs::remove_dir_all(dir).ok();
}

proptest! {
    #[test]
    fn maintained_btree_range_returns_every_pk(qtys in proptest::collection::vec(0_i64..1000, 1..24)) {
        let vault = AsterVault::with_clock(vault_id(), b"salt", FixedClock::new(10));
        let layer = RelationalLayer::new(&vault);
        let col = orders(vec![btree("qty_idx", "qty")]);
        let spec = qty_spec();
        for (idx, qty) in qtys.iter().enumerate() {
            layer
                .put_record(&col, &RecordKey::from_u64(idx as u64 + 1), &row("part", *qty))
                .unwrap();
        }
        let got = btree_range(
            &vault,
            &col,
            &spec,
            Some(&RecordValue::I64(0)),
            Some(&RecordValue::I64(999)),
            0,
        )
        .unwrap();
        let mut got_nums = got
            .iter()
            .map(|pk| u64::from_be_bytes(pk.as_bytes().try_into().unwrap()))
            .collect::<Vec<_>>();
        got_nums.sort_unstable();
        let expected = (1..=qtys.len() as u64).collect::<Vec<_>>();
        prop_assert_eq!(got_nums, expected);
    }
}

fn temp_dir(name: &str) -> PathBuf {
    let id = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("calyx-aster-{name}-{}-{id}", std::process::id()));
    fs::remove_dir_all(&dir).ok();
    dir
}
