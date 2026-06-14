//! FSV for #750: native `FieldType::U64` / `RecordValue::U64` btree indexes.
//!
//! Source of truth is the durable `index_btree` column family after flush and
//! reopen. The test decodes physical index keys and checks their field bytes.

use std::fs;
use std::path::PathBuf;

use calyx_aster::cf::ColumnFamily;
use calyx_aster::collection::{
    Collection, CollectionMode, DedupPolicy, FieldDef, FieldType, IsolationLevel, RetentionPolicy,
    Schema, SecondaryIndexKind, SecondaryIndexSpec, TemporalPolicy, TenantId, TxnPolicy,
    create_collection,
};
use calyx_aster::index::btree::btree_point;
use calyx_aster::index::{BtreeIndex, IndexId, IndexKind, IndexSpec};
use calyx_aster::layers::relational::collection_id;
use calyx_aster::layers::{KvLayer, RecordKey, RecordValue, RelationalLayer, Row};
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::VaultId;

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap()
}

fn root() -> PathBuf {
    std::env::var_os("CALYX_ISSUE750_U64_FSV_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("calyx-issue750-u64-index-fsv"))
}

fn index(field: &str) -> SecondaryIndexSpec {
    SecondaryIndexSpec {
        name: format!("{field}_idx"),
        kind: SecondaryIndexKind::Btree,
        fields: vec![field.to_string()],
    }
}

fn runtime_spec(name: &str, field: &str) -> IndexSpec {
    IndexSpec::new(
        IndexId::new(1),
        name,
        IndexKind::Btree,
        field,
        FieldType::U64,
    )
}

fn kv_collection() -> Collection {
    Collection {
        name: "issue750_kv_u64_ns".to_string(),
        mode: CollectionMode::KV,
        schema: None,
        panel: None,
        indexes: vec![index("ns")],
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

fn relational_collection() -> Collection {
    Collection {
        name: "issue750_rel_u64".to_string(),
        mode: CollectionMode::Records,
        schema: Some(Schema::SchemaFull(vec![FieldDef::new(
            "score",
            FieldType::U64,
            false,
        )])),
        panel: None,
        indexes: vec![index("score")],
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

fn decode_u64_order<C: calyx_core::Clock>(
    vault: &AsterVault<C>,
    col: &Collection,
    spec: &IndexSpec,
) -> Vec<u64> {
    let index = BtreeIndex::new(collection_id(col), spec.clone());
    let prefix = index.index_key_prefix();
    vault
        .scan_cf_at(vault.latest_seq(), ColumnFamily::IndexBtree)
        .unwrap()
        .into_iter()
        .filter(|(key, _)| key.starts_with(&prefix))
        .map(|(key, _)| {
            let (field, pk) = index.decode_index_key(&key).unwrap();
            let value = match field {
                RecordValue::U64(value) => value,
                other => panic!("decoded U64 index field as {other:?}"),
            };
            let field_bytes = &key[prefix.len()..key.len() - pk.as_bytes().len()];
            assert_eq!(field_bytes, &value.to_be_bytes());
            value
        })
        .collect()
}

#[test]
fn u64_indexes_survive_flush_reopen_and_sort_unsigned() {
    let dir = root().join("vault");
    fs::remove_dir_all(&dir).ok();
    let write_order = [u64::MAX, 0, (i64::MAX as u64) + 1, i64::MAX as u64, 1];
    let mut expected = write_order;
    expected.sort_unstable();

    let vault = AsterVault::new_durable(
        &dir,
        vault_id(),
        b"issue750-u64-index-fsv".to_vec(),
        VaultOptions::default(),
    )
    .unwrap();
    let kv = kv_collection();
    let rel = relational_collection();
    create_collection(&vault, kv.clone()).unwrap();
    create_collection(&vault, rel.clone()).unwrap();

    let kv_layer = KvLayer::new(&vault);
    let rel_layer = RelationalLayer::new(&vault);
    for (i, value) in write_order.into_iter().enumerate() {
        kv_layer.kv_set(&kv, value, b"k", b"v", None).unwrap();
        rel_layer
            .put_record(
                &rel,
                &RecordKey::from_u64((i + 1) as u64),
                &Row::new([("score", RecordValue::U64(value))]),
            )
            .unwrap();
    }

    vault.flush().unwrap();
    drop(vault);

    let reopened = AsterVault::open(
        &dir,
        vault_id(),
        b"issue750-u64-index-fsv".to_vec(),
        VaultOptions::default(),
    )
    .unwrap();
    let reopened_kv = KvLayer::new(&reopened);
    let reopened_rel = RelationalLayer::new(&reopened);
    let kv_spec = runtime_spec("ns_idx", "ns");
    let rel_spec = runtime_spec("score_idx", "score");
    let kv_max_value = reopened_kv
        .kv_get(&kv, u64::MAX, b"k")
        .unwrap()
        .expect("u64::MAX KV row must survive reopen");
    let rel_max_row = reopened_rel
        .get_record(&rel, &RecordKey::from_u64(1))
        .unwrap()
        .expect("u64::MAX relational row must survive reopen");
    assert_eq!(kv_max_value, b"v".to_vec());
    assert_eq!(rel_max_row.get("score"), Some(&RecordValue::U64(u64::MAX)));
    let kv_order = decode_u64_order(&reopened, &kv, &kv_spec);
    let rel_order = decode_u64_order(&reopened, &rel, &rel_spec);
    assert_eq!(kv_order, expected);
    assert_eq!(rel_order, expected);

    let max_ns_pks = btree_point(&reopened, &kv, &kv_spec, &RecordValue::U64(u64::MAX)).unwrap();
    assert_eq!(max_ns_pks.len(), 1);
    let max_score_pks =
        btree_point(&reopened, &rel, &rel_spec, &RecordValue::U64(u64::MAX)).unwrap();
    assert_eq!(max_score_pks, vec![RecordKey::from_u64(1)]);

    let artifact = serde_json::json!({
        "issue": 750,
        "source_of_truth": dir.join("cf").join("index_btree").display().to_string(),
        "write_order": write_order,
        "expected_unsigned_order": expected,
        "kv_schema_less_decoded_order": kv_order,
        "relational_schema_full_decoded_order": rel_order,
        "kv_max_ns_value_hex": hex(&kv_max_value),
        "rel_max_score_value": u64::MAX,
        "kv_max_ns_pk_hex": hex(max_ns_pks[0].as_bytes()),
        "rel_max_score_pk_hex": hex(max_score_pks[0].as_bytes()),
    });
    let out = root().join("issue750-u64-index-fsv-artifact.json");
    fs::write(&out, serde_json::to_vec_pretty(&artifact).unwrap()).unwrap();
    println!("{}", serde_json::to_string_pretty(&artifact).unwrap());
}

fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}
