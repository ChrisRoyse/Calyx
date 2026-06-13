use super::*;
use calyx_core::{FixedClock, LensId, SlotId, VaultId};
use proptest::prelude::*;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::vault::{AsterVault, VaultOptions};

static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap()
}

fn orders_collection() -> Collection {
    Collection {
        name: "orders".to_string(),
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

#[test]
fn create_collection_roundtrips_and_reads_after_reopen() {
    let dir = temp_dir("collection-roundtrip");
    let vault = AsterVault::new_durable(
        &dir,
        vault_id(),
        b"collection-salt".to_vec(),
        VaultOptions::default(),
    )
    .unwrap();
    let collection = orders_collection();
    let expected_key = collection_key("orders").unwrap();
    let expected_bytes = encode_collection(&collection).unwrap();

    create_collection(&vault, collection.clone()).unwrap();
    let raw = vault
        .read_cf_at(vault.latest_seq(), ColumnFamily::Collections, &expected_key)
        .unwrap()
        .unwrap();
    assert_eq!(raw, expected_bytes);
    assert_eq!(get_collection(&vault, "orders").unwrap(), collection);

    drop(vault);
    let reopened = AsterVault::open(
        &dir,
        vault_id(),
        b"collection-salt".to_vec(),
        VaultOptions::default(),
    )
    .unwrap();
    assert_eq!(get_collection(&reopened, "orders").unwrap(), collection);
    fs::remove_dir_all(dir).ok();
}

#[test]
fn collection_fsv_writes_durable_cf_readback() {
    let fsv_root = std::env::var_os("CALYX_FSV_ROOT").map(PathBuf::from);
    let dir = fsv_root
        .as_ref()
        .map(|root| root.join("vault"))
        .unwrap_or_else(|| temp_dir("collection-fsv"));
    fs::remove_dir_all(&dir).ok();

    let vault = AsterVault::new_durable(
        &dir,
        vault_id(),
        b"collection-fsv-salt".to_vec(),
        VaultOptions::default(),
    )
    .unwrap();
    let collection = orders_collection();
    let expected_key = collection_key("orders").unwrap();
    let expected_bytes = encode_collection(&collection).unwrap();
    let before_rows = collection_rows(&vault);
    assert!(before_rows.is_empty());

    create_collection(&vault, collection.clone()).unwrap();
    let after_rows = collection_rows(&vault);
    assert_eq!(
        after_rows,
        vec![(expected_key.clone(), expected_bytes.clone())]
    );
    vault.flush().unwrap();
    drop(vault);

    let reopened = AsterVault::open(
        &dir,
        vault_id(),
        b"collection-fsv-salt".to_vec(),
        VaultOptions::default(),
    )
    .unwrap();
    let reopened_collection = get_collection(&reopened, "orders").unwrap();
    let raw_after_reopen = reopened
        .read_cf_at(
            reopened.latest_seq(),
            ColumnFamily::Collections,
            &expected_key,
        )
        .unwrap()
        .unwrap();
    assert_eq!(reopened_collection, collection);
    assert_eq!(raw_after_reopen, expected_bytes);

    let edge_cases = vec![
        edge_result(&reopened, "empty_name", CALYX_INVALID_ARGUMENT, || {
            create_collection(
                &reopened,
                Collection {
                    name: String::new(),
                    ..orders_collection()
                },
            )
        }),
        edge_result(&reopened, "name_129_bytes", CALYX_INVALID_ARGUMENT, || {
            create_collection(
                &reopened,
                Collection {
                    name: "x".repeat(MAX_NAME_BYTES + 1),
                    ..orders_collection()
                },
            )
        }),
        edge_result(
            &reopened,
            "constellations_requires_panel",
            CALYX_INVALID_ARGUMENT,
            || {
                create_collection(
                    &reopened,
                    Collection {
                        mode: CollectionMode::Constellations,
                        panel: None,
                        ..orders_collection()
                    },
                )
            },
        ),
        edge_result(&reopened, "tau_above_one", CALYX_INVALID_ARGUMENT, || {
            create_collection(
                &reopened,
                Collection {
                    dedup: DedupPolicy::TctCosine {
                        required_slots: vec![SlotId::new(1).with_key("sem-self")],
                        tau: 1.1,
                        action: DedupAction::Reject,
                    },
                    ..orders_collection()
                },
            )
        }),
        edge_result(
            &reopened,
            "duplicate_orders",
            CALYX_COLLECTION_ALREADY_EXISTS,
            || create_collection(&reopened, orders_collection()),
        ),
        edge_result(
            &reopened,
            "missing_collection",
            CALYX_COLLECTION_NOT_FOUND,
            || get_collection(&reopened, "missing"),
        ),
    ];

    let readback = serde_json::json!({
        "issue": 450,
        "source_of_truth": dir.display().to_string(),
        "cf": ColumnFamily::Collections.name(),
        "key_ascii": "coll\\0orders",
        "key_hex": hex_bytes(&expected_key),
        "value_bytes": expected_bytes.len(),
        "value_blake3": blake3_hex(&expected_bytes),
        "before_rows": rows_json(&before_rows),
        "after_rows": rows_json(&after_rows),
        "cold_open_equal": reopened_collection == collection,
        "decoded_after_reopen": serde_json::to_value(&reopened_collection).unwrap(),
        "raw_after_reopen_value_blake3": blake3_hex(&raw_after_reopen),
        "edge_cases": edge_cases,
        "collections_cf_files": physical_files(&dir.join("cf").join("collections")),
    });
    assert_eq!(readback["cold_open_equal"], serde_json::json!(true));
    for edge in readback["edge_cases"].as_array().unwrap() {
        assert_eq!(edge["before_rows"], edge["after_rows"]);
    }

    if let Some(root) = fsv_root {
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("ph53-collection-readback.json"),
            serde_json::to_vec_pretty(&readback).unwrap(),
        )
        .unwrap();
        fs::write(root.join("collection-key.hex"), hex_bytes(&expected_key)).unwrap();
        fs::write(
            root.join("collection-value.hex"),
            hex_bytes(&expected_bytes),
        )
        .unwrap();
        write_blake3_sums(&root);
        println!("ph53_collection_fsv_root={}", root.display());
        println!("{}", serde_json::to_string_pretty(&readback).unwrap());
    } else {
        fs::remove_dir_all(dir).ok();
    }
}

#[test]
fn validation_edges_fail_closed_with_exact_codes() {
    assert_code(
        create_collection(
            &AsterVault::new(vault_id(), b"salt"),
            Collection {
                name: String::new(),
                ..orders_collection()
            },
        ),
        CALYX_INVALID_ARGUMENT,
    );
    assert_code(
        create_collection(
            &AsterVault::new(vault_id(), b"salt"),
            Collection {
                name: "x".repeat(129),
                ..orders_collection()
            },
        ),
        CALYX_INVALID_ARGUMENT,
    );
    assert_code(
        create_collection(
            &AsterVault::new(vault_id(), b"salt"),
            Collection {
                mode: CollectionMode::Constellations,
                panel: None,
                ..orders_collection()
            },
        ),
        CALYX_INVALID_ARGUMENT,
    );
    assert_code(
        create_collection(
            &AsterVault::new(vault_id(), b"salt"),
            Collection {
                dedup: DedupPolicy::TctCosine {
                    required_slots: vec![SlotId::new(1).with_key("sem-self")],
                    tau: 1.1,
                    action: DedupAction::Reject,
                },
                ..orders_collection()
            },
        ),
        CALYX_INVALID_ARGUMENT,
    );
    assert_code(
        create_collection(
            &AsterVault::new(vault_id(), b"salt"),
            Collection {
                schema: Some(Schema::SchemaFull(Vec::new())),
                ..orders_collection()
            },
        ),
        CALYX_INVALID_ARGUMENT,
    );
}

#[test]
fn duplicate_and_missing_collection_fail_closed() {
    let vault = AsterVault::with_clock(vault_id(), b"salt", FixedClock::new(10));
    create_collection(&vault, orders_collection()).unwrap();
    assert_code(
        create_collection(&vault, orders_collection()),
        CALYX_COLLECTION_ALREADY_EXISTS,
    );
    assert_code(
        get_collection(&vault, "missing"),
        CALYX_COLLECTION_NOT_FOUND,
    );
}

#[test]
fn constellation_collection_requires_non_empty_panel() {
    let mut collection = orders_collection();
    collection.mode = CollectionMode::Constellations;
    collection.panel = Some(PanelRef::new(LensId::from_bytes([7; 16])));
    collection.dedup = DedupPolicy::TctCosine {
        required_slots: vec![SlotId::new(1).with_key("sem-self")],
        tau: 0.72,
        action: DedupAction::RecurrenceSeries,
    };
    collection.validate().unwrap();

    collection.panel = Some(PanelRef {
        panel_version: 1,
        lenses: Vec::new(),
    });
    assert_code(collection.validate(), CALYX_INVALID_ARGUMENT);
}

proptest! {
    #[test]
    fn collection_bincode_roundtrips(mode in collection_mode_strategy()) {
        let mut collection = orders_collection();
        collection.mode = mode;
        if mode == CollectionMode::Constellations {
            collection.panel = Some(PanelRef::new(LensId::from_bytes([9; 16])));
        }
        let bytes = encode_collection(&collection).unwrap();
        prop_assert_eq!(decode_collection(&bytes).unwrap(), collection);
    }
}

fn collection_mode_strategy() -> impl Strategy<Value = CollectionMode> {
    prop_oneof![
        Just(CollectionMode::Records),
        Just(CollectionMode::Documents),
        Just(CollectionMode::KV),
        Just(CollectionMode::TimeSeries),
        Just(CollectionMode::Blob),
        Just(CollectionMode::Constellations),
    ]
}

fn assert_code<T: std::fmt::Debug>(result: Result<T>, code: &'static str) {
    let error = result.unwrap_err();
    assert_eq!(error.code, code);
}

fn edge_result<T: std::fmt::Debug>(
    vault: &AsterVault,
    case: &'static str,
    expected_code: &'static str,
    action: impl FnOnce() -> Result<T>,
) -> serde_json::Value {
    let before = collection_rows(vault);
    let error = action().unwrap_err();
    let after = collection_rows(vault);
    assert_eq!(error.code, expected_code);
    serde_json::json!({
        "case": case,
        "code": error.code,
        "before_rows": before.len(),
        "after_rows": after.len(),
        "message": error.message,
    })
}

fn collection_rows<C: calyx_core::Clock>(vault: &AsterVault<C>) -> Vec<(Vec<u8>, Vec<u8>)> {
    vault
        .scan_cf_at(vault.latest_seq(), ColumnFamily::Collections)
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
    for name in [
        "ph53-collection-readback.json",
        "collection-key.hex",
        "collection-value.hex",
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

fn temp_dir(name: &str) -> std::path::PathBuf {
    let id = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("calyx-aster-{name}-{}-{id}", std::process::id()));
    fs::remove_dir_all(&dir).ok();
    dir
}
