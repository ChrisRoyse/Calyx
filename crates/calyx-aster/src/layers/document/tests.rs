use super::codec::{DISC_DOCUMENT, DocumentCell, decode_cell, encode_cell, hex_bytes};
use super::errors::CALYX_SCHEMA_VIOLATION;
use super::*;
use crate::cf::ledger_key;
use crate::collection::{
    CALYX_INVALID_ARGUMENT, DedupPolicy, FieldDef, FieldType, IsolationLevel, RetentionPolicy,
    Schema, TemporalPolicy, TenantId, TxnPolicy, create_collection,
};
use crate::vault::VaultOptions;
use calyx_core::{FixedClock, VaultId};
use calyx_ledger::{EntryKind, SubjectId, decode as decode_ledger};
use proptest::prelude::*;
use serde_json::json;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap()
}

fn docs_collection() -> Collection {
    Collection {
        name: "docs".to_string(),
        mode: CollectionMode::Documents,
        schema: Some(Schema::SchemaLess),
        panel: None,
        indexes: Vec::new(),
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

fn typed_docs_collection() -> Collection {
    Collection {
        schema: Some(Schema::SchemaFull(vec![
            FieldDef::new("title", FieldType::Text, false),
            FieldDef::new("rank", FieldType::I64, false),
            FieldDef::new("note", FieldType::Text, true),
        ])),
        ..docs_collection()
    }
}

#[test]
fn nested_doc_roundtrips_and_subtree_uses_tuple_prefix() {
    let vault = AsterVault::with_clock(vault_id(), b"salt", FixedClock::new(10));
    let layer = DocumentLayer::new(&vault);
    let col = docs_collection();
    let doc_id = DocId::from_text("d1");
    let doc = json!({"a":{"b":42},"c":7});

    let seq = layer.put_doc(&col, doc_id, &doc).unwrap();
    let key = document_key(&col, doc_id, &["a", "b"]).unwrap();
    assert_eq!(key[0], DISC_DOCUMENT);
    assert_eq!(&key[1..9], &collection_id(&col).to_be_bytes());
    assert_eq!(&key[9..25], doc_id.as_bytes());
    assert_eq!(&key[25..], &[1, b'a', 1, b'b']);
    assert_eq!(layer.get_doc(&col, doc_id).unwrap(), Some(doc.clone()));
    assert_eq!(
        layer.get_subtree(&col, doc_id, &["a"]).unwrap(),
        Some(json!({"b":42}))
    );
    assert_eq!(layer.get_subtree(&col, doc_id, &["missing"]).unwrap(), None);
    let ledger = vault
        .read_cf_at(seq, ColumnFamily::Ledger, &ledger_key(0))
        .unwrap()
        .unwrap();
    assert_document_ledger_entry(&ledger, 0, &col, doc_id);
}

#[test]
fn put_replaces_stale_paths_and_delete_tombstones_visible_doc() {
    let vault = AsterVault::with_clock(vault_id(), b"salt", FixedClock::new(10));
    let layer = DocumentLayer::new(&vault);
    let col = docs_collection();
    let doc_id = DocId::from_text("replace");
    layer
        .put_doc(&col, doc_id, &json!({"a":{"b":42},"c":7}))
        .unwrap();
    layer.put_doc(&col, doc_id, &json!({"a":1})).unwrap();

    assert_eq!(layer.get_doc(&col, doc_id).unwrap(), Some(json!({"a":1})));
    assert_eq!(layer.get_subtree(&col, doc_id, &["c"]).unwrap(), None);
    layer.delete_doc(&col, doc_id).unwrap();
    assert_eq!(layer.get_doc(&col, doc_id).unwrap(), None);
    assert!(
        document_rows(&vault)
            .into_iter()
            .any(|(_, value)| matches!(decode_cell(&value).unwrap(), DocumentCell::Tombstone))
    );
}

#[test]
fn schemafull_top_level_validation_and_edges_fail_closed() {
    let vault = AsterVault::with_clock(vault_id(), b"salt", FixedClock::new(10));
    let layer = DocumentLayer::new(&vault);
    let col = typed_docs_collection();
    let doc_id = DocId::from_text("typed");

    assert_eq!(layer.get_doc(&col, doc_id).unwrap(), None);
    assert_eq!(
        layer
            .put_doc(&col, doc_id, &json!({"title":"Ada"}))
            .unwrap_err()
            .code,
        CALYX_SCHEMA_VIOLATION
    );
    assert_eq!(
        layer
            .put_doc(&col, doc_id, &json!({"title":"Ada","rank":1,"extra":true}))
            .unwrap_err()
            .code,
        CALYX_SCHEMA_VIOLATION
    );
    assert_eq!(
        document_key(&col, doc_id, &[&"x".repeat(256)])
            .unwrap_err()
            .code,
        CALYX_INVALID_ARGUMENT
    );
    let corrupt = DocId::from_text("corrupt");
    vault
        .write_cf(
            ColumnFamily::Document,
            document_key(&col, corrupt, &["x"]).unwrap(),
            vec![0],
        )
        .unwrap();
    assert_eq!(
        layer.get_doc(&col, corrupt).unwrap_err().code,
        "CALYX_ASTER_CORRUPT_SHARD"
    );
}

#[test]
fn layer_trait_put_get_and_range_json_bytes() {
    let vault = AsterVault::with_clock(vault_id(), b"salt", FixedClock::new(10));
    let layer = DocumentLayer::new(&vault);
    let col = docs_collection();
    let first = DocId::from_bytes([1; 16]);
    let second = DocId::from_bytes([2; 16]);

    layer
        .put(&col, first.as_bytes(), br#"{"x":1}"#)
        .expect("trait put first");
    layer
        .put(&col, second.as_bytes(), br#"{"x":2}"#)
        .expect("trait put second");
    assert_eq!(
        serde_json::from_slice::<Value>(&layer.get(&col, first.as_bytes()).unwrap().unwrap())
            .unwrap(),
        json!({"x":1})
    );
    let docs = layer
        .range(
            &col,
            first.as_bytes(),
            DocId::from_bytes([3; 16]).as_bytes(),
            10,
        )
        .unwrap();
    assert_eq!(docs.len(), 2);
}

#[test]
fn durable_document_fsv_writes_readback_artifacts() {
    let fsv_root = std::env::var_os("CALYX_FSV_ROOT").map(PathBuf::from);
    let dir = fsv_root
        .as_ref()
        .map(|root| root.join("vault"))
        .unwrap_or_else(|| temp_dir("document-fsv"));
    fs::remove_dir_all(&dir).ok();
    let vault = AsterVault::new_durable(
        &dir,
        vault_id(),
        b"document-fsv-salt".to_vec(),
        VaultOptions::default(),
    )
    .unwrap();
    let layer = DocumentLayer::new(&vault);
    let col = docs_collection();
    let doc_id = DocId::from_text("d1");
    let doc = json!({"a":{"b":42},"c":7});
    let before_rows = document_rows(&vault);
    assert!(before_rows.is_empty());

    create_collection(&vault, col.clone()).unwrap();
    let seq = layer.put_doc(&col, doc_id, &doc).unwrap();
    let expected_key = document_key(&col, doc_id, &["a", "b"]).unwrap();
    let expected_value = encode_cell(&DocumentCell::leaf(json!(42)).unwrap()).unwrap();
    let after_put_rows = document_rows(&vault);
    assert_eq!(after_put_rows.len(), 2);
    assert!(after_put_rows.contains(&(expected_key.clone(), expected_value.clone())));
    assert_eq!(layer.get_doc(&col, doc_id).unwrap(), Some(doc.clone()));
    assert_eq!(
        layer.get_subtree(&col, doc_id, &["a"]).unwrap(),
        Some(json!({"b":42}))
    );
    let edge_cases = vec![
        edge_absent_doc(&layer, &col),
        edge_absent_subtree(&layer, &col, doc_id),
        edge_delete_doc(&layer, &col),
        edge_long_segment(&col, doc_id),
        edge_corrupt_value(&vault, &layer, &col),
    ];
    vault.flush().unwrap();
    drop(vault);

    let reopened = AsterVault::open(
        &dir,
        vault_id(),
        b"document-fsv-salt".to_vec(),
        VaultOptions::default(),
    )
    .unwrap();
    let reopened_layer = DocumentLayer::new(&reopened);
    let raw_after_reopen = reopened
        .read_cf_at(reopened.latest_seq(), ColumnFamily::Document, &expected_key)
        .unwrap()
        .unwrap();
    assert_eq!(raw_after_reopen, expected_value);
    assert_eq!(
        reopened_layer.get_doc(&col, doc_id).unwrap(),
        Some(doc.clone())
    );
    let subtree = reopened_layer.get_subtree(&col, doc_id, &["a"]).unwrap();
    assert_eq!(subtree, Some(json!({"b":42})));
    let readback = serde_json::json!({
        "issue": 452,
        "source_of_truth": dir.display().to_string(),
        "cf": ColumnFamily::Document.name(),
        "collection_id": collection_id(&col),
        "doc_id_hex": hex_bytes(doc_id.as_bytes()),
        "key_hex": hex_bytes(&expected_key),
        "key_first_byte_hex": hex_bytes(&expected_key[..1]),
        "value_hex": hex_bytes(&expected_value),
        "value_blake3": blake3_hex(&expected_value),
        "before_rows": rows_json(&before_rows),
        "after_put_rows": rows_json(&after_put_rows),
        "cold_open_equal": raw_after_reopen == expected_value,
        "decoded_after_reopen": reopened_layer.get_doc(&col, doc_id).unwrap().unwrap(),
        "subtree_a": subtree.unwrap(),
        "subtree_a_has_c": false,
        "mvcc_put_seq": seq,
        "edge_cases": edge_cases,
        "document_cf_files": physical_files(&dir.join("cf").join("document")),
    });
    assert_eq!(readback["key_first_byte_hex"], json!("02"));
    assert_eq!(readback["cold_open_equal"], json!(true));

    if let Some(root) = fsv_root {
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("ph53-document-readback.json"),
            serde_json::to_vec_pretty(&readback).unwrap(),
        )
        .unwrap();
        fs::write(root.join("document-key.hex"), hex_bytes(&expected_key)).unwrap();
        fs::write(root.join("document-value.hex"), hex_bytes(&expected_value)).unwrap();
        write_blake3_sums(&root);
        println!("ph53_document_fsv_root={}", root.display());
        println!("{}", serde_json::to_string_pretty(&readback).unwrap());
    } else {
        fs::remove_dir_all(dir).ok();
    }
}

proptest! {
    #[test]
    fn flat_object_documents_roundtrip(fields in prop::collection::btree_map("[a-z]{1,8}", any::<i64>(), 1..=5)) {
        let vault = AsterVault::with_clock(vault_id(), b"salt", FixedClock::new(10));
        let layer = DocumentLayer::new(&vault);
        let col = docs_collection();
        let doc_id = DocId::from_text("proptest");
        let doc = serde_json::to_value(fields).unwrap();
        layer.put_doc(&col, doc_id, &doc).unwrap();
        prop_assert_eq!(layer.get_doc(&col, doc_id).unwrap(), Some(doc));
    }
}

fn edge_absent_doc<C: Clock>(layer: &DocumentLayer<C>, col: &Collection) -> Value {
    let before = document_rows(layer.vault).len();
    let got = layer.get_doc(col, DocId::from_text("absent")).unwrap();
    let after = document_rows(layer.vault).len();
    assert_eq!(got, None);
    json!({"case":"absent_doc","result":"none","before_rows":before,"after_rows":after})
}

fn edge_absent_subtree<C: Clock>(
    layer: &DocumentLayer<C>,
    col: &Collection,
    doc_id: DocId,
) -> Value {
    let before = document_rows(layer.vault).len();
    let got = layer.get_subtree(col, doc_id, &["z"]).unwrap();
    let after = document_rows(layer.vault).len();
    assert_eq!(got, None);
    json!({"case":"absent_subtree","result":"none","before_rows":before,"after_rows":after})
}

fn edge_delete_doc<C: Clock>(layer: &DocumentLayer<C>, col: &Collection) -> Value {
    let doc_id = DocId::from_text("delete-edge");
    layer.put_doc(col, doc_id, &json!({"x":1})).unwrap();
    let before = document_rows(layer.vault).len();
    layer.delete_doc(col, doc_id).unwrap();
    let after = document_rows(layer.vault).len();
    assert_eq!(layer.get_doc(col, doc_id).unwrap(), None);
    json!({"case":"delete_doc","result":"none","before_rows":before,"after_rows":after})
}

fn edge_long_segment(col: &Collection, doc_id: DocId) -> Value {
    let error = document_key(col, doc_id, &[&"x".repeat(256)]).unwrap_err();
    assert_eq!(error.code, CALYX_INVALID_ARGUMENT);
    json!({"case":"long_segment","code":error.code})
}

fn edge_corrupt_value<C: Clock>(
    vault: &AsterVault<C>,
    layer: &DocumentLayer<C>,
    col: &Collection,
) -> Value {
    let before = document_rows(vault).len();
    let doc_id = DocId::from_text("corrupt-edge");
    vault
        .write_cf(
            ColumnFamily::Document,
            document_key(col, doc_id, &["x"]).unwrap(),
            vec![0],
        )
        .unwrap();
    let error = layer.get_doc(col, doc_id).unwrap_err();
    let after = document_rows(vault).len();
    assert_eq!(error.code, "CALYX_ASTER_CORRUPT_SHARD");
    json!({"case":"corrupt_value","code":error.code,"before_rows":before,"after_rows":after})
}

fn document_rows<C: Clock>(vault: &AsterVault<C>) -> Vec<(Vec<u8>, Vec<u8>)> {
    vault
        .scan_cf_at(vault.latest_seq(), ColumnFamily::Document)
        .unwrap()
}

fn assert_document_ledger_entry(bytes: &[u8], seq: u64, col: &Collection, doc_id: DocId) {
    let entry = decode_ledger(bytes).unwrap();
    assert_eq!(entry.seq, seq);
    assert_eq!(entry.kind, EntryKind::Ingest);
    assert_eq!(entry.subject, ledger_subject(&document_prefix(col, doc_id)));
    assert!(matches!(entry.subject, SubjectId::Query(_)));
    let payload: Value = serde_json::from_slice(&entry.payload).unwrap();
    assert_eq!(
        payload["collection_id"],
        json!(format!("{:016x}", collection_id(col)))
    );
    assert_eq!(payload["doc_id"], json!(hex_bytes(doc_id.as_bytes())));
    assert!(payload["doc_hash"].as_str().unwrap().len() == 64);
    assert!(payload["rows_hash"].as_str().unwrap().len() == 64);
}

fn rows_json(rows: &[(Vec<u8>, Vec<u8>)]) -> Vec<Value> {
    rows.iter()
        .map(|(key, value)| {
            json!({
                "key_hex": hex_bytes(key),
                "value_bytes": value.len(),
                "value_blake3": blake3_hex(value),
            })
        })
        .collect()
}

fn physical_files(dir: &Path) -> Vec<Value> {
    let mut files = Vec::new();
    if dir.exists() {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            let bytes = fs::read(&path).unwrap();
            files.push(json!({
                "path": path.display().to_string(),
                "bytes": bytes.len(),
                "blake3": blake3_hex(&bytes),
            }));
        }
    }
    files.sort_by_key(|file| file["path"].as_str().unwrap_or_default().to_string());
    files
}

fn write_blake3_sums(root: &Path) {
    let mut entries = Vec::new();
    for name in [
        "ph53-document-readback.json",
        "document-key.hex",
        "document-value.hex",
    ] {
        let bytes = fs::read(root.join(name)).unwrap();
        entries.push(format!("{}  {name}", blake3_hex(&bytes)));
    }
    fs::write(root.join("blake3-sums.txt"), entries.join("\n")).unwrap();
}

fn blake3_hex(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

fn temp_dir(name: &str) -> PathBuf {
    let id = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("calyx-aster-{name}-{}-{id}", std::process::id()));
    fs::remove_dir_all(&dir).ok();
    dir
}
