use super::*;
use calyx_core::{Clock, FixedClock, LensId, SlotId, VaultId};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::cf::ColumnFamily;
use crate::layers::{KvLayer, RecordKey, RecordValue, RelationalLayer, Row};
use crate::vault::{AsterVault, VaultOptions};

static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap()
}

fn lens_id() -> LensId {
    LensId::from_parts("sem-self", b"weights", b"corpus", b"structured")
}

fn other_lens_id() -> LensId {
    LensId::from_parts("lex", b"weights", b"corpus", b"structured")
}

fn records_collection(name: &str) -> Collection {
    Collection {
        name: name.to_string(),
        mode: CollectionMode::Records,
        schema: Some(Schema::SchemaFull(vec![FieldDef::new(
            "pk",
            FieldType::I64,
            false,
        )])),
        panel: None,
        indexes: Vec::new(),
        dedup: DedupPolicy::Off,
        temporal: TemporalPolicy::default(),
        retention: RetentionPolicy::Forever,
        txn_policy: TxnPolicy::default(),
        tenant: TenantId::default(),
    }
}

fn kv_collection(name: &str) -> Collection {
    Collection {
        name: name.to_string(),
        mode: CollectionMode::KV,
        schema: None,
        panel: None,
        indexes: Vec::new(),
        dedup: DedupPolicy::Off,
        temporal: TemporalPolicy::default(),
        retention: RetentionPolicy::Forever,
        txn_policy: TxnPolicy::default(),
        tenant: TenantId::default(),
    }
}

#[test]
fn plain_records_write_does_not_touch_slot_cf() {
    let vault = AsterVault::with_clock(vault_id(), b"salt", FixedClock::new(10));
    let collection = records_collection("orders");
    create_collection(&vault, collection.clone()).unwrap();
    let layer = RelationalLayer::new(&vault);

    for pk in 1..=3 {
        layer
            .put_record(
                &collection,
                &RecordKey::from_u64(pk),
                &Row::new([("pk", RecordValue::I64(pk as i64))]),
            )
            .unwrap();
    }

    assert!(slot_rows(&vault).is_empty());
    assert_eq!(
        vault
            .scan_cf_at(vault.latest_seq(), ColumnFamily::Relational)
            .unwrap()
            .len(),
        3
    );
}

#[test]
fn add_lens_sets_constellations_mode_and_backfill_marker() {
    let vault = AsterVault::with_clock(vault_id(), b"salt", FixedClock::new(20));
    let lens = lens_id();
    create_collection(&vault, records_collection("orders")).unwrap();
    register_lens(&vault, lens).unwrap();

    add_lens(&vault, "orders", lens).unwrap();

    let upgraded = get_collection(&vault, "orders").unwrap();
    assert_eq!(upgraded.mode, CollectionMode::Constellations);
    assert!(collection_has_lens(&upgraded));
    assert_eq!(upgraded.panel.unwrap().lenses, vec![lens]);
    assert!(online_row(&vault, &backfill_pending_key("orders").unwrap()).is_some());
}

#[test]
fn post_upgrade_records_write_routes_to_slot_zero() {
    let vault = AsterVault::with_clock(vault_id(), b"salt", FixedClock::new(30));
    let lens = lens_id();
    create_collection(&vault, records_collection("orders")).unwrap();
    register_lens(&vault, lens).unwrap();
    add_lens(&vault, "orders", lens).unwrap();

    let upgraded = get_collection(&vault, "orders").unwrap();
    RelationalLayer::new(&vault)
        .put_record(
            &upgraded,
            &RecordKey::from_u64(7),
            &Row::new([("pk", RecordValue::I64(7))]),
        )
        .unwrap();

    assert_eq!(slot_rows(&vault).len(), 1);
    assert!(
        vault
            .scan_cf_at(vault.latest_seq(), ColumnFamily::Relational)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn add_lens_edges_fail_closed_and_kv_upgrades() {
    let vault = AsterVault::with_clock(vault_id(), b"salt", FixedClock::new(40));
    let lens = lens_id();
    create_collection(&vault, records_collection("orders")).unwrap();
    register_lens(&vault, lens).unwrap();
    add_lens(&vault, "orders", lens).unwrap();
    assert_code(
        add_lens(&vault, "orders", lens),
        CALYX_COLLECTION_LENS_DUPLICATE,
    );

    create_collection(&vault, records_collection("unknowns")).unwrap();
    assert_code(
        add_lens(&vault, "unknowns", other_lens_id()),
        CALYX_LENS_NOT_FOUND,
    );
    assert_eq!(
        get_collection(&vault, "unknowns").unwrap().mode,
        CollectionMode::Records
    );
    assert!(online_row(&vault, &backfill_pending_key("unknowns").unwrap()).is_none());

    create_collection(&vault, kv_collection("cache")).unwrap();
    register_lens(&vault, other_lens_id()).unwrap();
    add_lens(&vault, "cache", other_lens_id()).unwrap();
    let upgraded = get_collection(&vault, "cache").unwrap();
    assert_eq!(upgraded.mode, CollectionMode::Constellations);
    KvLayer::new(&vault)
        .kv_set(&upgraded, 0, b"k", b"v", None)
        .unwrap();
    assert_eq!(slot_rows(&vault).len(), 1);
}

#[test]
fn add_lens_wal_failure_keeps_old_mode_and_no_marker() {
    let dir = temp_dir("progressive-fail-closed");
    let vault = AsterVault::new_durable(
        &dir,
        vault_id(),
        b"durable-salt".to_vec(),
        VaultOptions::default(),
    )
    .unwrap();
    let lens = lens_id();
    create_collection(&vault, records_collection("orders")).unwrap();
    register_lens(&vault, lens).unwrap();

    vault.fail_next_wal_append_for_test();
    assert!(add_lens(&vault, "orders", lens).is_err());
    assert_eq!(
        get_collection(&vault, "orders").unwrap().mode,
        CollectionMode::Records
    );
    assert!(online_row(&vault, &backfill_pending_key("orders").unwrap()).is_none());

    drop(vault);
    let reopened = AsterVault::open(
        &dir,
        vault_id(),
        b"durable-salt".to_vec(),
        VaultOptions::default(),
    )
    .unwrap();
    assert_eq!(
        get_collection(&reopened, "orders").unwrap().mode,
        CollectionMode::Records
    );
    assert!(online_row(&reopened, &backfill_pending_key("orders").unwrap()).is_none());
    fs::remove_dir_all(dir).ok();
}

#[test]
fn progressive_enhancement_fsv_writes_durable_readback() {
    let fsv_root = std::env::var_os("CALYX_FSV_ROOT").map(PathBuf::from);
    let dir = fsv_root
        .as_ref()
        .map(|root| root.join("vault"))
        .unwrap_or_else(|| temp_dir("progressive-fsv"));
    fs::remove_dir_all(&dir).ok();

    let vault = AsterVault::new_durable(
        &dir,
        vault_id(),
        b"progressive-fsv-salt".to_vec(),
        VaultOptions::default(),
    )
    .unwrap();
    let lens = lens_id();
    let collection = records_collection("orders");
    create_collection(&vault, collection.clone()).unwrap();
    let layer = RelationalLayer::new(&vault);
    layer
        .put_record(
            &collection,
            &RecordKey::from_u64(1),
            &Row::new([("pk", RecordValue::I64(1))]),
        )
        .unwrap();
    let before_slot_rows = slot_rows(&vault);
    let before_marker = online_row(&vault, &backfill_pending_key("orders").unwrap());

    register_lens(&vault, lens).unwrap();
    add_lens(&vault, "orders", lens).unwrap();
    let upgraded = get_collection(&vault, "orders").unwrap();
    layer
        .put_record(
            &upgraded,
            &RecordKey::from_u64(2),
            &Row::new([("pk", RecordValue::I64(2))]),
        )
        .unwrap();
    let marker_key = backfill_pending_key("orders").unwrap();
    let marker_value = online_row(&vault, &marker_key).unwrap();
    let after_slot_rows = slot_rows(&vault);
    let edge_cases = progressive_edge_cases(&vault);
    let collections_rows = vault
        .scan_cf_at(vault.latest_seq(), ColumnFamily::Collections)
        .unwrap();
    let online_rows = vault
        .scan_cf_at(vault.latest_seq(), ColumnFamily::Online)
        .unwrap();
    vault.flush().unwrap();
    drop(vault);

    let reopened = AsterVault::open(
        &dir,
        vault_id(),
        b"progressive-fsv-salt".to_vec(),
        VaultOptions::default(),
    )
    .unwrap();
    let reopened_collection = get_collection(&reopened, "orders").unwrap();
    let readback = serde_json::json!({
        "issue": 455,
        "source_of_truth": dir.display().to_string(),
        "collection_key_hex": hex_bytes(&collection_key("orders").unwrap()),
        "backfill_key_hex": hex_bytes(&marker_key),
        "backfill_value_utf8": String::from_utf8(marker_value).unwrap(),
        "before_slot_00_rows": before_slot_rows.len(),
        "before_backfill_marker": before_marker.is_some(),
        "after_slot_00_rows": after_slot_rows.len(),
        "decoded_after_upgrade": serde_json::to_value(&upgraded).unwrap(),
        "decoded_after_reopen": serde_json::to_value(&reopened_collection).unwrap(),
        "edge_cases": edge_cases,
        "collections_rows": rows_json(&collections_rows),
        "online_rows": rows_json(&online_rows),
        "collections_cf_files": physical_files(&dir.join("cf").join("collections")),
        "online_cf_files": physical_files(&dir.join("cf").join("online")),
        "relational_cf_files": physical_files(&dir.join("cf").join("relational")),
        "slot_00_cf_files": physical_files(&dir.join("cf").join("slot_00")),
    });
    assert_eq!(readback["before_slot_00_rows"], serde_json::json!(0));
    assert_eq!(readback["before_backfill_marker"], serde_json::json!(false));
    assert_eq!(readback["after_slot_00_rows"], serde_json::json!(1));
    assert_eq!(reopened_collection.mode, CollectionMode::Constellations);

    if let Some(root) = fsv_root {
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("ph53-progressive-readback.json"),
            serde_json::to_vec_pretty(&readback).unwrap(),
        )
        .unwrap();
        fs::write(root.join("backfill-key.hex"), hex_bytes(&marker_key)).unwrap();
        write_blake3_sums(&root);
        println!("ph53_progressive_fsv_root={}", root.display());
        println!("{}", serde_json::to_string_pretty(&readback).unwrap());
    } else {
        fs::remove_dir_all(dir).ok();
    }
}

fn progressive_edge_cases<C: Clock>(vault: &AsterVault<C>) -> Vec<serde_json::Value> {
    let lens = lens_id();
    let other_lens = other_lens_id();

    let duplicate_before = get_collection(vault, "orders").unwrap();
    let duplicate_before_marker = online_row(vault, &backfill_pending_key("orders").unwrap());
    let duplicate_error = add_lens(vault, "orders", lens).unwrap_err();
    let duplicate_after = get_collection(vault, "orders").unwrap();
    let duplicate_after_marker = online_row(vault, &backfill_pending_key("orders").unwrap());
    assert_eq!(duplicate_error.code, CALYX_COLLECTION_LENS_DUPLICATE);
    assert_eq!(duplicate_before, duplicate_after);
    assert_eq!(duplicate_before_marker, duplicate_after_marker);

    create_collection(vault, records_collection("unknown_edge")).unwrap();
    let unknown_before = get_collection(vault, "unknown_edge").unwrap();
    let unknown_before_marker = online_row(vault, &backfill_pending_key("unknown_edge").unwrap());
    let unknown_error = add_lens(vault, "unknown_edge", other_lens).unwrap_err();
    let unknown_after = get_collection(vault, "unknown_edge").unwrap();
    let unknown_after_marker = online_row(vault, &backfill_pending_key("unknown_edge").unwrap());
    assert_eq!(unknown_error.code, CALYX_LENS_NOT_FOUND);
    assert_eq!(unknown_before, unknown_after);
    assert_eq!(unknown_before_marker, unknown_after_marker);

    create_collection(vault, kv_collection("cache_edge")).unwrap();
    let kv_before = get_collection(vault, "cache_edge").unwrap();
    register_lens(vault, other_lens).unwrap();
    add_lens(vault, "cache_edge", other_lens).unwrap();
    let kv_after = get_collection(vault, "cache_edge").unwrap();
    KvLayer::new(vault)
        .kv_set(&kv_after, 0, b"k", b"v", None)
        .unwrap();

    vec![
        serde_json::json!({
            "case": "duplicate_same_lens",
            "code": duplicate_error.code,
            "before_mode": format!("{:?}", duplicate_before.mode),
            "after_mode": format!("{:?}", duplicate_after.mode),
            "before_marker": duplicate_before_marker.is_some(),
            "after_marker": duplicate_after_marker.is_some(),
        }),
        serde_json::json!({
            "case": "unknown_lens",
            "code": unknown_error.code,
            "before_mode": format!("{:?}", unknown_before.mode),
            "after_mode": format!("{:?}", unknown_after.mode),
            "before_marker": unknown_before_marker.is_some(),
            "after_marker": unknown_after_marker.is_some(),
        }),
        serde_json::json!({
            "case": "kv_upgrade",
            "before_mode": format!("{:?}", kv_before.mode),
            "after_mode": format!("{:?}", kv_after.mode),
            "after_marker": online_row(vault, &backfill_pending_key("cache_edge").unwrap())
                .is_some(),
            "slot_00_rows_after_kv_write": slot_rows(vault).len(),
        }),
    ]
}

fn assert_code<T: std::fmt::Debug>(result: Result<T>, code: &'static str) {
    let error = result.unwrap_err();
    assert_eq!(error.code, code);
}

fn slot_rows<C: Clock>(vault: &AsterVault<C>) -> Vec<(Vec<u8>, Vec<u8>)> {
    vault
        .scan_cf_at(vault.latest_seq(), ColumnFamily::slot(SlotId::new(0)))
        .unwrap()
}

fn online_row<C: Clock>(vault: &AsterVault<C>, key: &[u8]) -> Option<Vec<u8>> {
    vault
        .read_cf_at(vault.latest_seq(), ColumnFamily::Online, key)
        .unwrap()
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
    collect_files(dir, &mut files);
    files.sort_by_key(|file| file["path"].as_str().unwrap_or_default().to_string());
    files
}

fn collect_files(dir: &Path, files: &mut Vec<serde_json::Value>) {
    if !dir.exists() {
        return;
    }
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            collect_files(&path, files);
        } else {
            let bytes = fs::read(&path).unwrap();
            files.push(serde_json::json!({
                "path": path.display().to_string(),
                "bytes": bytes.len(),
                "blake3": blake3_hex(&bytes),
            }));
        }
    }
}

fn write_blake3_sums(root: &Path) {
    let mut entries = Vec::new();
    for name in ["ph53-progressive-readback.json", "backfill-key.hex"] {
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
