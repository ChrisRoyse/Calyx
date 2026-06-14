use std::fs;
use std::path::{Path, PathBuf};

use calyx_aster::cf::ColumnFamily;
use calyx_aster::collection::{
    Collection, CollectionMode, DedupPolicy, FieldDef, FieldType, RetentionPolicy, Schema,
    TemporalPolicy, TenantId, TxnPolicy, create_collection,
};
use calyx_aster::layers::{
    BlobId, BlobLayer, DocId, DocumentLayer, KvLayer, RecordKey, RecordValue, RelationalLayer,
    RollupWindow, Row, TimeSeriesLayer,
};
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{SlotId, VaultId};
use serde_json::{Value, json};

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap()
}

#[test]
fn ph53_all_paradigm_roundtrips() {
    let fsv_root = std::env::var_os("CALYX_FSV_ROOT").map(PathBuf::from);
    let dir = fsv_root
        .as_ref()
        .map(|root| root.join("ph53-vault"))
        .unwrap_or_else(|| std::env::temp_dir().join("calyx-ph53-fsv-test"));
    fs::remove_dir_all(&dir).ok();

    let salt = b"ph53-fsv-salt".to_vec();
    let options = VaultOptions::default();
    let vault = AsterVault::new_durable(&dir, vault_id(), salt.clone(), options.clone()).unwrap();
    let cols = Collections::new();
    create_all_collections(&vault, &cols);
    write_all_layers(&vault, &cols);
    let before = read_all_layers(&vault, &cols);
    vault.flush().unwrap();
    drop(vault);

    let reopened = AsterVault::open(&dir, vault_id(), salt, options).unwrap();
    let after = read_all_layers(&reopened, &cols);
    assert_eq!(after, before);
    assert_eq!(after["relational_order_qty"], json!(3));
    assert_eq!(after["relational_range_len"], json!(1));
    assert_eq!(after["joined_product_name"], json!("bolt"));
    assert_eq!(after["document_subtree"], json!({"author":"alice"}));
    assert_eq!(after["kv_value_hex"], json!("3432"));
    assert_eq!(after["ts_range_len"], json!(3));
    assert_eq!(after["ts_rollup_count"], json!(3));
    assert_eq!(after["ts_rollup_sum"], json!(6.0));
    assert_eq!(after["blob_byte_exact"], json!(true));
    assert_eq!(after["slot_00_rows"], json!(0));
    assert_eq!(after["edges"]["missing_doc_subtree"], Value::Null);
    assert_eq!(after["edges"]["inverted_range_len"], json!(0));
    assert_eq!(after["edges"]["absent_kv"], Value::Null);

    let readback = json!({
        "issue": 456,
        "source_of_truth": dir.display().to_string(),
        "before_restart": before,
        "after_restart": after,
        "cf_files": physical_files(&dir.join("cf")),
    });
    println!("ph53 FSV: all paradigm round-trips PASS");
    println!("{}", serde_json::to_string_pretty(&readback).unwrap());

    if let Some(root) = fsv_root {
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("ph53-all-paradigm-readback.json"),
            serde_json::to_vec_pretty(&readback).unwrap(),
        )
        .unwrap();
    } else {
        fs::remove_dir_all(dir).ok();
    }
}

struct Collections {
    orders: Collection,
    products: Collection,
    docs: Collection,
    cache: Collection,
    metrics: Collection,
    assets: Collection,
}

impl Collections {
    fn new() -> Self {
        Self {
            orders: collection(
                "orders",
                CollectionMode::Records,
                Some(Schema::SchemaFull(vec![
                    FieldDef::new("product_id", FieldType::I64, false),
                    FieldDef::new("qty", FieldType::I64, false),
                ])),
            ),
            products: collection(
                "products",
                CollectionMode::Records,
                Some(Schema::SchemaFull(vec![
                    FieldDef::new("name", FieldType::Text, false),
                    FieldDef::new("price_cents", FieldType::I64, false),
                ])),
            ),
            docs: collection("docs", CollectionMode::Documents, Some(Schema::SchemaLess)),
            cache: collection("cache", CollectionMode::KV, Some(Schema::SchemaLess)),
            metrics: collection(
                "metrics",
                CollectionMode::TimeSeries,
                Some(Schema::SchemaLess),
            ),
            assets: collection("assets", CollectionMode::Blob, Some(Schema::SchemaLess)),
        }
    }
}

fn collection(name: &str, mode: CollectionMode, schema: Option<Schema>) -> Collection {
    Collection {
        name: name.to_string(),
        mode,
        schema,
        panel: None,
        indexes: Vec::new(),
        dedup: DedupPolicy::Off,
        temporal: TemporalPolicy::default(),
        retention: RetentionPolicy::Forever,
        txn_policy: TxnPolicy::default(),
        tenant: TenantId::default(),
    }
}

fn create_all_collections(vault: &AsterVault, cols: &Collections) {
    for col in [
        &cols.orders,
        &cols.products,
        &cols.docs,
        &cols.cache,
        &cols.metrics,
        &cols.assets,
    ] {
        create_collection(vault, col.clone()).unwrap();
    }
}

fn write_all_layers(vault: &AsterVault, cols: &Collections) {
    let rel = RelationalLayer::new(vault);
    rel.put_record(
        &cols.orders,
        &RecordKey::from_u64(1),
        &Row::new([
            ("product_id", RecordValue::I64(42)),
            ("qty", RecordValue::I64(3)),
        ]),
    )
    .unwrap();
    rel.put_record(
        &cols.products,
        &RecordKey::from_u64(42),
        &Row::new([
            ("name", RecordValue::Text("bolt".to_string())),
            ("price_cents", RecordValue::I64(100)),
        ]),
    )
    .unwrap();

    let doc = DocumentLayer::new(vault);
    doc.put_doc(
        &cols.docs,
        DocId::from_text("d1"),
        &json!({"meta":{"author":"alice"},"body":"hello"}),
    )
    .unwrap();

    KvLayer::new(vault)
        .kv_set(&cols.cache, 1, b"x", b"42", None)
        .unwrap();

    let ts = TimeSeriesLayer::new(vault);
    for (idx, val) in [1.0, 2.0, 3.0].into_iter().enumerate() {
        ts.ts_write(&cols.metrics, 7, 1_000_000_000 + idx as u64, val)
            .unwrap();
    }

    BlobLayer::new(vault)
        .blob_put(&cols.assets, BlobId::from_text("b1"), &blob_payload())
        .unwrap();
}

fn read_all_layers(vault: &AsterVault, cols: &Collections) -> Value {
    let rel = RelationalLayer::new(vault);
    let order = rel
        .get_record(&cols.orders, &RecordKey::from_u64(1))
        .unwrap()
        .unwrap();
    let range = rel
        .range(
            &cols.orders,
            &RecordKey::from_u64(0),
            &RecordKey::from_u64(100),
            10,
        )
        .unwrap();
    let joined = rel
        .join_by_ref(
            &cols.orders,
            &RecordKey::from_u64(1),
            "products",
            "product_id",
        )
        .unwrap()
        .unwrap();

    let doc = DocumentLayer::new(vault);
    let doc_id = DocId::from_text("d1");
    let subtree = doc.get_subtree(&cols.docs, doc_id, &["meta"]).unwrap();
    let missing_subtree = doc.get_subtree(&cols.docs, doc_id, &["missing"]).unwrap();

    let kv = KvLayer::new(vault);
    let kv_value = kv.kv_get(&cols.cache, 1, b"x").unwrap().unwrap();
    let absent_kv = kv.kv_get(&cols.cache, 1, b"absent").unwrap();

    let ts = TimeSeriesLayer::new(vault);
    let ts_range = ts.ts_range(&cols.metrics, 7, 0, u64::MAX).unwrap();
    let rollup = ts
        .ts_rollup(&cols.metrics, 7, RollupWindow::OneHour, 1_000_000_000)
        .unwrap()
        .unwrap();

    let blob = BlobLayer::new(vault);
    let payload = blob_payload();
    let blob_read = blob
        .blob_get(&cols.assets, BlobId::from_text("b1"))
        .unwrap()
        .unwrap();
    let inverted_range = rel
        .range(
            &cols.orders,
            &RecordKey::from_u64(100),
            &RecordKey::from_u64(0),
            10,
        )
        .unwrap();
    let slot_rows = vault
        .scan_cf_at(vault.latest_seq(), ColumnFamily::slot(SlotId::new(0)))
        .unwrap()
        .len();

    json!({
        "relational_order_qty": i64_field(&order, "qty"),
        "relational_range_len": range.len(),
        "joined_product_name": text_field(&joined, "name"),
        "document_subtree": subtree,
        "kv_value_hex": hex_bytes(&kv_value),
        "ts_range": ts_range,
        "ts_range_len": ts_range.len(),
        "ts_rollup_count": rollup.count,
        "ts_rollup_sum": rollup.sum,
        "blob_len": blob_read.len(),
        "blob_hash": blake3::hash(&blob_read).to_hex().to_string(),
        "blob_byte_exact": blob_read == payload,
        "slot_00_rows": slot_rows,
        "edges": {
            "missing_doc_subtree": missing_subtree,
            "inverted_range_len": inverted_range.len(),
            "absent_kv": absent_kv,
        }
    })
}

fn blob_payload() -> Vec<u8> {
    b"calyx-ph53-blob-test".repeat(512)
}

fn i64_field(row: &Row, field: &str) -> i64 {
    match row.get(field) {
        Some(RecordValue::I64(value)) => *value,
        other => panic!("field {field} expected I64, got {other:?}"),
    }
}

fn text_field(row: &Row, field: &str) -> String {
    match row.get(field) {
        Some(RecordValue::Text(value)) => value.clone(),
        other => panic!("field {field} expected Text, got {other:?}"),
    }
}

fn physical_files(dir: &Path) -> Vec<Value> {
    let mut files = Vec::new();
    if dir.exists() {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                files.extend(physical_files(&path));
            } else {
                files.push(json!({
                    "path": path.display().to_string(),
                    "bytes": fs::read(&path).unwrap().len(),
                }));
            }
        }
    }
    files.sort_by_key(|file| file["path"].as_str().unwrap_or_default().to_string());
    files
}

fn hex_bytes(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}
