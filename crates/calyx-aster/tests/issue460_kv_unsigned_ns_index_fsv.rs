//! FSV for PH54 T04 follow-up: an unindexed KV collection accepts the full
//! `u64` namespace, and a `ns`-indexed collection stores those namespaces in
//! correct **unsigned** order on disk.
//!
//! This is the regression guard for the bug where `kv_set` unconditionally
//! coerced `ns` into an `i64` (rejecting every `ns >= 2^63`) and - had the
//! coercion not failed - would have sorted `u64::MAX` (`-1` as `i64`) *before*
//! `0`. The fix now encodes a schema-less namespace index as native `U64`, whose
//! on-disk big-endian bytes match the natural unsigned order.
//!
//! Source of truth: the durable `index_btree` column family, read back after a
//! flush + reopen (not return values).

use std::fs;
use std::path::PathBuf;

use calyx_aster::cf::ColumnFamily;
use calyx_aster::collection::{
    Collection, CollectionMode, DedupPolicy, FieldType, IsolationLevel, RetentionPolicy,
    SecondaryIndexKind, SecondaryIndexSpec, TemporalPolicy, TenantId, TxnPolicy, create_collection,
};
use calyx_aster::index::{BtreeIndex, IndexId, IndexKind, IndexSpec};
use calyx_aster::layers::KvLayer;
use calyx_aster::layers::relational::collection_id;
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::VaultId;

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap()
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
        txn_policy: TxnPolicy {
            isolation: IsolationLevel::ReadCommitted,
            cost_cap_ms: None,
        },
        tenant: TenantId::default(),
    }
}

fn ns_index() -> SecondaryIndexSpec {
    SecondaryIndexSpec {
        name: "ns_idx".to_string(),
        kind: SecondaryIndexKind::Btree,
        fields: vec!["ns".to_string()],
    }
}

/// Runtime spec matching exactly how `IndexMaintenance` builds the `ns` index:
/// collection-scoped by the relational collection id, ordinal+1 id, and the
/// schema-less `U64` field type.
fn ns_spec() -> IndexSpec {
    IndexSpec::new(
        IndexId::new(1),
        "ns_idx",
        IndexKind::Btree,
        "ns",
        FieldType::U64,
    )
}

fn root() -> PathBuf {
    std::env::var_os("CALYX_ISSUE460_NS_FSV_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("calyx-issue460-ns-index-fsv"))
}

#[test]
fn unindexed_kv_accepts_full_u64_namespace() {
    // Synthetic boundary namespaces: 0, 1, i64::MAX, i64::MAX+1 (the old failure
    // threshold), and u64::MAX. With no index declared every one must round-trip.
    let cases: [u64; 5] = [0, 1, i64::MAX as u64, (i64::MAX as u64) + 1, u64::MAX];

    let vault = AsterVault::with_clock(vault_id(), b"salt", calyx_core::FixedClock::new(1000));
    let layer = KvLayer::new(&vault);
    let col = kv_collection("sessions_unindexed", Vec::new());

    for ns in cases {
        layer
            .kv_set(&col, ns, b"k", format!("v-{ns}").as_bytes(), None)
            .unwrap_or_else(|e| panic!("kv_set ns={ns} must succeed, got {e:?}"));
        // Source of truth: read the value straight back out of the KV CF path.
        let got = layer.kv_get(&col, ns, b"k").unwrap();
        assert_eq!(
            got,
            Some(format!("v-{ns}").into_bytes()),
            "ns={ns} value must read back from the KV column family"
        );
    }

    // No index declared: no index_btree rows written for any namespace.
    let index_rows = vault
        .scan_cf_at(vault.latest_seq(), ColumnFamily::IndexBtree)
        .unwrap();
    assert!(
        index_rows.is_empty(),
        "unindexed KV must not write index_btree rows, found {}",
        index_rows.len()
    );
}

#[test]
fn ns_index_orders_namespaces_unsigned_on_disk() {
    let dir = root().join("vault");
    fs::remove_dir_all(&dir).ok();

    // Synthetic namespaces written OUT of order on purpose. Sorted unsigned they
    // are: 0 < 1 < i64::MAX < i64::MAX+1 < u64::MAX. The old i64 encoding would
    // have placed i64::MAX+1 and u64::MAX (negative as i64) first; this proves
    // it does not.
    let write_order: [u64; 5] = [u64::MAX, 0, (i64::MAX as u64) + 1, i64::MAX as u64, 1];
    let mut expected_unsigned = write_order;
    expected_unsigned.sort_unstable();

    let vault = AsterVault::new_durable(
        &dir,
        vault_id(),
        b"issue460-ns-index-fsv".to_vec(),
        VaultOptions::default(),
    )
    .unwrap();
    let layer = KvLayer::new(&vault);
    let col = kv_collection("sessions_ns_indexed", vec![ns_index()]);
    create_collection(&vault, col.clone()).unwrap();

    for ns in write_order {
        layer.kv_set(&col, ns, b"k", b"v", None).unwrap();
    }

    // Durability boundary: flush, drop, reopen, then read the source of truth.
    vault.flush().unwrap();
    drop(vault);
    let reopened = AsterVault::open(
        &dir,
        vault_id(),
        b"issue460-ns-index-fsv".to_vec(),
        VaultOptions::default(),
    )
    .unwrap();

    let spec = ns_spec();
    let index = BtreeIndex::new(collection_id(&col), spec.clone());

    // scan_cf_at returns keys in stored byte order. The decoded sequence MUST
    // equal ascending unsigned ns.
    let rows = reopened
        .scan_cf_at(reopened.latest_seq(), ColumnFamily::IndexBtree)
        .unwrap();
    assert_eq!(
        rows.len(),
        write_order.len(),
        "one index_btree row per namespace must survive flush+reopen"
    );

    let mut on_disk_order = Vec::new();
    for (key, _empty) in &rows {
        let (field_val, pk) = index.decode_index_key(key).unwrap();
        let ns = match field_val {
            calyx_aster::layers::RecordValue::U64(value) => value,
            other => panic!("ns index value must decode as U64, got {other:?}"),
        };
        let field_bytes = &key[13..key.len() - pk.as_bytes().len()];
        assert_eq!(field_bytes, &ns.to_be_bytes());
        on_disk_order.push(ns);
    }

    assert_eq!(
        on_disk_order, expected_unsigned,
        "index_btree must store namespaces in ascending UNSIGNED order"
    );

    // Evidence artifact: the actual decoded on-disk ordering.
    let artifact = serde_json::json!({
        "issue": 460,
        "property": "ns index stores u64 namespaces in unsigned order",
        "source_of_truth": dir.join("cf").join("index_btree").display().to_string(),
        "write_order": write_order,
        "expected_unsigned_order": expected_unsigned,
        "on_disk_decoded_order": on_disk_order,
        "old_i64_would_have_ordered": {
            "note": "u64::MAX and i64::MAX+1 reinterpret as negative i64 and would sort first",
        },
    });
    let out = root().join("issue460-ns-index-fsv-artifact.json");
    fs::write(&out, serde_json::to_vec_pretty(&artifact).unwrap()).unwrap();
    println!("{}", serde_json::to_string_pretty(&artifact).unwrap());
}
