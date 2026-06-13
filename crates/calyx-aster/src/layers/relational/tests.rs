use super::*;
use crate::cf::ledger_key;
use calyx_core::{FixedClock, VaultId};
use calyx_ledger::{EntryKind, SubjectId, decode as decode_ledger};
use proptest::prelude::*;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::collection::{
    DedupPolicy, FieldDef, IsolationLevel, RetentionPolicy, TemporalPolicy, TenantId, TxnPolicy,
    create_collection,
};
use crate::vault::VaultOptions;

static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap()
}

fn orders_collection() -> Collection {
    Collection {
        name: "orders".to_string(),
        mode: CollectionMode::Records,
        schema: Some(Schema::SchemaFull(vec![
            FieldDef::new("item", FieldType::Text, false),
            FieldDef::new("qty", FieldType::I64, false),
            FieldDef::new("customer_pk", FieldType::I64, true),
        ])),
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

fn customers_collection() -> Collection {
    Collection {
        name: "customers".to_string(),
        ..orders_collection()
    }
}

fn order_row(item: &str, qty: i64) -> Row {
    Row::new([
        ("item", RecordValue::Text(item.to_string())),
        ("qty", RecordValue::I64(qty)),
    ])
}

#[test]
fn put_get_and_range_use_big_endian_record_keys() {
    let vault = AsterVault::with_clock(vault_id(), b"salt", FixedClock::new(10));
    let layer = RelationalLayer::new(&vault);
    let col = orders_collection();
    let pk = RecordKey::from_u64(1);
    let row = order_row("bolt", 5);

    let seq = layer.put_record(&col, &pk, &row).unwrap();
    let key = record_key(&col, &pk).unwrap();
    assert_eq!(key[0], DISC_RECORD);
    assert_eq!(&key[1..9], &collection_id(&col).to_be_bytes());
    assert_eq!(&key[9..11], &8_u16.to_be_bytes());
    assert_eq!(&key[11..19], &1_u64.to_be_bytes());
    assert_eq!(layer.get_record(&col, &pk).unwrap(), Some(row.clone()));
    let ledger = vault
        .read_cf_at(seq, ColumnFamily::Ledger, &ledger_key(0))
        .unwrap()
        .unwrap();
    assert_relational_ledger_entry(
        &ledger,
        0,
        &col,
        &pk,
        &key,
        &encode_record_value(&row).unwrap(),
    );

    for pk in [3_u64, 5, 7, 9] {
        layer
            .put_record(
                &col,
                &RecordKey::from_u64(pk),
                &order_row("bolt", pk as i64),
            )
            .unwrap();
    }
    let rows = layer
        .range(&col, &RecordKey::from_u64(0), &RecordKey::from_u64(100), 10)
        .unwrap();
    assert_eq!(rows.len(), 5);
    assert_eq!(rows[0], row);
    assert_eq!(rows[4].get("qty"), Some(&RecordValue::I64(9)));
}

#[test]
fn joins_by_reference_with_one_snapshot() {
    let vault = AsterVault::with_clock(vault_id(), b"salt", FixedClock::new(10));
    let layer = RelationalLayer::new(&vault);
    let orders = orders_collection();
    let customers = customers_collection();
    create_collection(&vault, orders.clone()).unwrap();
    create_collection(&vault, customers.clone()).unwrap();
    let customer = order_row("ada", 1);
    let order = Row::new([
        ("item", RecordValue::Text("bolt".to_string())),
        ("qty", RecordValue::I64(5)),
        ("customer_pk", RecordValue::I64(42)),
    ]);
    layer
        .put_record(&customers, &RecordKey::from_u64(42), &customer)
        .unwrap();
    layer
        .put_record(&orders, &RecordKey::from_u64(1), &order)
        .unwrap();

    assert_eq!(
        layer
            .join_by_ref(&orders, &RecordKey::from_u64(1), "customers", "customer_pk")
            .unwrap(),
        Some(customer)
    );
}

#[test]
fn edges_fail_closed_with_exact_codes() {
    let vault = AsterVault::with_clock(vault_id(), b"salt", FixedClock::new(10));
    let layer = RelationalLayer::new(&vault);
    let col = orders_collection();

    assert_eq!(
        layer.get_record(&col, &RecordKey::from_u64(404)).unwrap(),
        None
    );
    assert_eq!(
        layer
            .put_record(
                &col,
                &RecordKey::from_u64(2),
                &Row::new([("item", RecordValue::Text("bolt".to_string()))])
            )
            .unwrap_err()
            .code,
        CALYX_SCHEMA_VIOLATION
    );
    assert!(
        layer
            .range(&col, &RecordKey::from_u64(100), &RecordKey::from_u64(0), 10)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        decode_record_value(&[0]).unwrap_err().code,
        "CALYX_ASTER_CORRUPT_SHARD"
    );
}

#[test]
fn durable_relational_fsv_writes_readback_artifacts() {
    let fsv_root = std::env::var_os("CALYX_FSV_ROOT").map(PathBuf::from);
    let dir = fsv_root
        .as_ref()
        .map(|root| root.join("vault"))
        .unwrap_or_else(|| temp_dir("relational-fsv"));
    fs::remove_dir_all(&dir).ok();
    let vault = AsterVault::new_durable(
        &dir,
        vault_id(),
        b"relational-fsv-salt".to_vec(),
        VaultOptions::default(),
    )
    .unwrap();
    let layer = RelationalLayer::new(&vault);
    let col = orders_collection();
    let pk = RecordKey::from_u64(1);
    let row = order_row("bolt", 5);
    let before_rows = relational_rows(&vault);
    assert!(before_rows.is_empty());

    create_collection(&vault, col.clone()).unwrap();
    let seq = layer.put_record(&col, &pk, &row).unwrap();
    let expected_key = record_key(&col, &pk).unwrap();
    let expected_value = encode_record_value(&row).unwrap();
    let expected_ledger_key = ledger_key(0);
    let after_put_rows = relational_rows(&vault);
    assert_eq!(
        after_put_rows,
        vec![(expected_key.clone(), expected_value.clone())]
    );
    let after_ledger_rows = ledger_rows(&vault);
    assert_eq!(after_ledger_rows.len(), 1);
    assert_eq!(after_ledger_rows[0].0, expected_ledger_key);
    let expected_ledger_value = after_ledger_rows[0].1.clone();
    assert_relational_ledger_entry(
        &expected_ledger_value,
        0,
        &col,
        &pk,
        &expected_key,
        &expected_value,
    );
    assert_eq!(layer.get_record(&col, &pk).unwrap(), Some(row.clone()));
    assert_eq!(
        layer
            .range(&col, &RecordKey::from_u64(0), &RecordKey::from_u64(100), 10)
            .unwrap(),
        vec![row.clone()]
    );

    let edge_cases = vec![
        edge_read_none(&layer, &col),
        edge_schema_violation(&layer, &col),
        edge_inverted_range(&layer, &col),
        edge_corrupt_value(&vault, &layer, &col),
    ];
    vault.flush().unwrap();
    drop(vault);

    let reopened = AsterVault::open(
        &dir,
        vault_id(),
        b"relational-fsv-salt".to_vec(),
        VaultOptions::default(),
    )
    .unwrap();
    let reopened_layer = RelationalLayer::new(&reopened);
    let raw_after_reopen = reopened
        .read_cf_at(
            reopened.latest_seq(),
            ColumnFamily::Relational,
            &expected_key,
        )
        .unwrap()
        .unwrap();
    let ledger_after_reopen = reopened
        .read_cf_at(
            reopened.latest_seq(),
            ColumnFamily::Ledger,
            &expected_ledger_key,
        )
        .unwrap()
        .unwrap();
    assert_eq!(raw_after_reopen, expected_value);
    assert_eq!(ledger_after_reopen, expected_ledger_value);
    assert_eq!(
        reopened_layer.get_record(&col, &pk).unwrap(),
        Some(row.clone())
    );

    let readback = serde_json::json!({
        "issue": 451,
        "source_of_truth": dir.display().to_string(),
        "cf": ColumnFamily::Relational.name(),
        "collection_id": collection_id(&col),
        "key_hex": hex_bytes(&expected_key),
        "value_hex": hex_bytes(&expected_value),
        "value_blake3": blake3_hex(&expected_value),
        "before_rows": rows_json(&before_rows),
        "after_put_rows": rows_json(&after_put_rows),
        "after_ledger_rows": rows_json(&after_ledger_rows),
        "cold_open_equal": raw_after_reopen == expected_value,
        "ledger_cold_open_equal": ledger_after_reopen == expected_ledger_value,
        "decoded_after_reopen": serde_json::to_value(reopened_layer.get_record(&col, &pk).unwrap().unwrap()).unwrap(),
        "range_0_100_len": reopened_layer.range(&col, &RecordKey::from_u64(0), &RecordKey::from_u64(100), 10).unwrap().len(),
        "mvcc_put_seq": seq,
        "ledger_entry_key_hex": hex_bytes(&expected_ledger_key),
        "ledger_entry_value_hex": hex_bytes(&expected_ledger_value),
        "edge_cases": edge_cases,
        "relational_cf_files": physical_files(&dir.join("cf").join("relational")),
    });
    assert_eq!(readback["cold_open_equal"], serde_json::json!(true));
    assert_eq!(readback["ledger_cold_open_equal"], serde_json::json!(true));

    if let Some(root) = fsv_root {
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("ph53-relational-readback.json"),
            serde_json::to_vec_pretty(&readback).unwrap(),
        )
        .unwrap();
        fs::write(root.join("relational-key.hex"), hex_bytes(&expected_key)).unwrap();
        fs::write(
            root.join("relational-value.hex"),
            hex_bytes(&expected_value),
        )
        .unwrap();
        write_blake3_sums(&root);
        println!("ph53_relational_fsv_root={}", root.display());
        println!("{}", serde_json::to_string_pretty(&readback).unwrap());
    } else {
        fs::remove_dir_all(dir).ok();
    }
}

proptest! {
    #[test]
    fn put_then_get_roundtrips(pk in any::<u64>(), qty in 0_i64..10_000) {
        let vault = AsterVault::with_clock(vault_id(), b"salt", FixedClock::new(10));
        let layer = RelationalLayer::new(&vault);
        let col = orders_collection();
        let key = RecordKey::from_u64(pk);
        let row = order_row("part", qty);
        layer.put_record(&col, &key, &row).unwrap();
        prop_assert_eq!(layer.get_record(&col, &key).unwrap(), Some(row));
    }
}

fn edge_read_none<C: Clock>(layer: &RelationalLayer<C>, col: &Collection) -> serde_json::Value {
    let before = relational_rows(layer.vault).len();
    let got = layer.get_record(col, &RecordKey::from_u64(404)).unwrap();
    let after = relational_rows(layer.vault).len();
    assert_eq!(got, None);
    serde_json::json!({"case":"absent_pk","result":"none","before_rows":before,"after_rows":after})
}

fn edge_schema_violation<C: Clock>(
    layer: &RelationalLayer<C>,
    col: &Collection,
) -> serde_json::Value {
    let before = relational_rows(layer.vault).len();
    let error = layer
        .put_record(
            col,
            &RecordKey::from_u64(2),
            &Row::new([("item", RecordValue::Text("bolt".to_string()))]),
        )
        .unwrap_err();
    let after = relational_rows(layer.vault).len();
    assert_eq!(error.code, CALYX_SCHEMA_VIOLATION);
    assert_eq!(before, after);
    serde_json::json!({"case":"missing_required_qty","code":error.code,"before_rows":before,"after_rows":after})
}

fn edge_inverted_range<C: Clock>(
    layer: &RelationalLayer<C>,
    col: &Collection,
) -> serde_json::Value {
    let before = relational_rows(layer.vault).len();
    let rows = layer
        .range(col, &RecordKey::from_u64(100), &RecordKey::from_u64(0), 10)
        .unwrap();
    let after = relational_rows(layer.vault).len();
    assert!(rows.is_empty());
    serde_json::json!({"case":"start_greater_than_end","result_len":rows.len(),"before_rows":before,"after_rows":after})
}

fn edge_corrupt_value<C: Clock>(
    vault: &AsterVault<C>,
    layer: &RelationalLayer<C>,
    col: &Collection,
) -> serde_json::Value {
    let before = relational_rows(vault).len();
    let corrupt_pk = RecordKey::from_u64(200);
    vault
        .write_cf(
            ColumnFamily::Relational,
            record_key(col, &corrupt_pk).unwrap(),
            vec![0],
        )
        .unwrap();
    let error = layer.get_record(col, &corrupt_pk).unwrap_err();
    let after = relational_rows(vault).len();
    assert_eq!(error.code, "CALYX_ASTER_CORRUPT_SHARD");
    serde_json::json!({"case":"corrupt_value","code":error.code,"before_rows":before,"after_rows":after})
}

fn relational_rows<C: Clock>(vault: &AsterVault<C>) -> Vec<(Vec<u8>, Vec<u8>)> {
    vault
        .scan_cf_at(vault.latest_seq(), ColumnFamily::Relational)
        .unwrap()
}

fn ledger_rows<C: Clock>(vault: &AsterVault<C>) -> Vec<(Vec<u8>, Vec<u8>)> {
    vault
        .scan_cf_at(vault.latest_seq(), ColumnFamily::Ledger)
        .unwrap()
}

fn assert_relational_ledger_entry(
    bytes: &[u8],
    seq: u64,
    col: &Collection,
    pk: &RecordKey,
    key: &[u8],
    value: &[u8],
) {
    let entry = decode_ledger(bytes).unwrap();
    assert_eq!(entry.seq, seq);
    assert_eq!(entry.kind, EntryKind::Ingest);
    assert_eq!(entry.subject, ledger_subject(key));
    assert!(matches!(entry.subject, SubjectId::Query(_)));
    let payload: serde_json::Value = serde_json::from_slice(&entry.payload).unwrap();
    assert_eq!(
        payload["collection_id"],
        serde_json::json!(format!("{:016x}", collection_id(col)))
    );
    assert_eq!(
        payload["pk_hash"],
        serde_json::json!(blake3::hash(pk.as_bytes()).to_hex().to_string())
    );
    assert_eq!(
        payload["record_hash"],
        serde_json::json!(blake3::hash(key).to_hex().to_string())
    );
    assert_eq!(
        payload["value_hash"],
        serde_json::json!(blake3::hash(value).to_hex().to_string())
    );
}

fn rows_json(rows: &[(Vec<u8>, Vec<u8>)]) -> Vec<serde_json::Value> {
    rows.iter()
        .map(|(key, value)| {
            serde_json::json!({
                "key_hex": hex_bytes(key),
                "value_bytes": value.len(),
                "value_blake3": blake3_hex(value),
            })
        })
        .collect()
}

fn physical_files(dir: &Path) -> Vec<serde_json::Value> {
    let mut files = Vec::new();
    if dir.exists() {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            let bytes = fs::read(&path).unwrap();
            files.push(serde_json::json!({
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
        "ph53-relational-readback.json",
        "relational-key.hex",
        "relational-value.hex",
    ] {
        let bytes = fs::read(root.join(name)).unwrap();
        entries.push(format!("{}  {name}", blake3_hex(&bytes)));
    }
    fs::write(root.join("blake3-sums.txt"), entries.join("\n")).unwrap();
}

fn hex_bytes(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(hex_digit(byte >> 4));
        out.push(hex_digit(byte & 0x0f));
    }
    out
}

fn hex_digit(value: u8) -> char {
    match value {
        0..=9 => char::from(b'0' + value),
        10..=15 => char::from(b'a' + value - 10),
        _ => unreachable!("nibble out of range"),
    }
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
