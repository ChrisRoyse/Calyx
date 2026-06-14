use super::*;

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use calyx_core::{FixedClock, Ts, VaultId};
use proptest::prelude::*;

use crate::collection::{
    DedupPolicy, RetentionPolicy, SecondaryIndexKind, SecondaryIndexSpec, TemporalPolicy, TenantId,
    TxnPolicy, create_collection,
};
use crate::vault::VaultOptions;

static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

/// Clock whose `now()` can be advanced by the test driver, for TTL proofs.
#[derive(Clone)]
struct AdvanceableClock(Arc<AtomicU64>);

impl AdvanceableClock {
    fn new(ts: Ts) -> Self {
        Self(Arc::new(AtomicU64::new(ts)))
    }
    fn set(&self, ts: Ts) {
        self.0.store(ts, Ordering::SeqCst);
    }
}

impl Clock for AdvanceableClock {
    fn now(&self) -> Ts {
        self.0.load(Ordering::SeqCst)
    }
}

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap()
}

fn kv_collection() -> Collection {
    Collection {
        name: "sessions".to_string(),
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
fn set_get_roundtrip_uses_0x03_discriminant() {
    let vault = AsterVault::with_clock(vault_id(), b"salt", FixedClock::new(1000));
    let layer = KvLayer::new(&vault);
    let col = kv_collection();

    layer.kv_set(&col, 1, b"foo", b"bar", None).unwrap();

    let key = kv_key(&col, 1, b"foo");
    assert_eq!(key[0], DISC_KV, "KV key must carry the 0x03 discriminant");
    assert_eq!(&key[1..9], &collection_id(&col).to_be_bytes());
    assert_eq!(&key[9..17], &1_u64.to_be_bytes());
    assert_eq!(&key[17..19], &3_u16.to_be_bytes());
    assert_eq!(&key[19..], b"foo");

    assert_eq!(
        layer.kv_get(&col, 1, b"foo").unwrap(),
        Some(b"bar".to_vec())
    );
    // Namespace scoping: same user key in a different ns is independent.
    assert_eq!(layer.kv_get(&col, 2, b"foo").unwrap(), None);
}

#[test]
fn large_namespace_does_not_block_key_index_maintenance() {
    let vault = AsterVault::with_clock(vault_id(), b"salt", FixedClock::new(1000));
    let layer = KvLayer::new(&vault);
    let mut col = kv_collection();
    col.indexes.push(SecondaryIndexSpec {
        name: "ns_idx".to_string(),
        kind: SecondaryIndexKind::Btree,
        fields: vec!["ns".to_string()],
    });
    col.indexes.push(SecondaryIndexSpec {
        name: "key_idx".to_string(),
        kind: SecondaryIndexKind::Btree,
        fields: vec!["key".to_string()],
    });

    layer
        .kv_set(&col, u64::MAX, b"max-ns", b"value", None)
        .unwrap();

    assert_eq!(
        layer.kv_get(&col, u64::MAX, b"max-ns").unwrap(),
        Some(b"value".to_vec())
    );
    assert_eq!(
        vault
            .scan_cf_at(vault.latest_seq(), ColumnFamily::IndexBtree)
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn ttl_expires_check_on_read() {
    let clock = AdvanceableClock::new(1000);
    let vault = AsterVault::with_clock(vault_id(), b"salt", clock.clone());
    let layer = KvLayer::new(&vault);
    let col = kv_collection();

    layer
        .kv_set(
            &col,
            0,
            b"tok",
            b"live",
            Some(std::time::Duration::from_millis(5)),
        )
        .unwrap();
    // expires_at = 1000 + 5 = 1005; still visible at 1000 and 1004.
    assert_eq!(
        layer.kv_get(&col, 0, b"tok").unwrap(),
        Some(b"live".to_vec())
    );
    clock.set(1004);
    assert_eq!(
        layer.kv_get(&col, 0, b"tok").unwrap(),
        Some(b"live".to_vec())
    );
    // At/after expires_at the record is invisible — but the bytes remain.
    clock.set(1005);
    assert_eq!(layer.kv_get(&col, 0, b"tok").unwrap(), None);
    clock.set(9999);
    assert_eq!(layer.kv_get(&col, 0, b"tok").unwrap(), None);
    let raw = vault
        .read_cf_at(
            vault.latest_seq(),
            ColumnFamily::Kv,
            &kv_key(&col, 0, b"tok"),
        )
        .unwrap()
        .expect("expired bytes still physically present until PH58 janitor");
    let (expires_at, payload) = decode_value(&raw).unwrap();
    assert_eq!(expires_at, 1005);
    assert_eq!(payload, b"live");
}

#[test]
fn sub_millisecond_ttl_fails_loud() {
    let vault = AsterVault::with_clock(vault_id(), b"salt", FixedClock::new(1000));
    let layer = KvLayer::new(&vault);
    let col = kv_collection();
    // Clock resolution is Unix ms; a nanosecond TTL cannot be honored, so we
    // error loudly rather than silently store an immortal record.
    let error = layer
        .kv_set(
            &col,
            0,
            b"k",
            b"v",
            Some(std::time::Duration::from_nanos(1)),
        )
        .unwrap_err();
    assert_eq!(error.code, CALYX_INVALID_ARGUMENT);
    assert_eq!(layer.kv_get(&col, 0, b"k").unwrap(), None);
}

#[test]
fn delete_hides_key_via_native_tombstone() {
    let vault = AsterVault::with_clock(vault_id(), b"salt", FixedClock::new(1000));
    let layer = KvLayer::new(&vault);
    let col = kv_collection();

    layer.kv_set(&col, 7, b"k", b"v", None).unwrap();
    assert_eq!(layer.kv_get(&col, 7, b"k").unwrap(), Some(b"v".to_vec()));
    layer.kv_delete(&col, 7, b"k").unwrap();
    assert_eq!(layer.kv_get(&col, 7, b"k").unwrap(), None);
    // A re-set after delete makes the key live again.
    layer.kv_set(&col, 7, b"k", b"v2", None).unwrap();
    assert_eq!(layer.kv_get(&col, 7, b"k").unwrap(), Some(b"v2".to_vec()));
}

#[test]
fn range_returns_sorted_live_entries_in_namespace() {
    let vault = AsterVault::with_clock(vault_id(), b"salt", FixedClock::new(1000));
    let layer = KvLayer::new(&vault);
    let col = kv_collection();

    for (k, v) in [(b"a".as_slice(), b"1"), (b"c", b"3"), (b"b", b"2")] {
        layer.kv_set(&col, 1, k, v, None).unwrap();
    }
    layer.kv_set(&col, 1, b"z", b"99", None).unwrap();
    layer.kv_delete(&col, 1, b"b").unwrap();

    let rows = layer.kv_range(&col, 1, b"a", b"z", 10).unwrap();
    assert_eq!(
        rows,
        vec![
            (b"a".to_vec(), b"1".to_vec()),
            (b"c".to_vec(), b"3".to_vec())
        ]
    );
}

#[test]
fn edge_cases_fail_closed_with_exact_codes() {
    let vault = AsterVault::with_clock(vault_id(), b"salt", FixedClock::new(1000));
    let layer = KvLayer::new(&vault);
    let col = kv_collection();

    // (1) absent key reads back as None.
    assert_eq!(layer.kv_get(&col, 0, b"missing").unwrap(), None);

    // (2) empty user key is rejected.
    assert_eq!(
        layer.kv_set(&col, 0, b"", b"v", None).unwrap_err().code,
        CALYX_INVALID_ARGUMENT
    );

    // (3) wrong collection mode is rejected.
    let mut wrong = col.clone();
    wrong.mode = CollectionMode::Records;
    assert_eq!(
        layer.kv_get(&wrong, 0, b"k").unwrap_err().code,
        CALYX_INVALID_ARGUMENT
    );

    // (4) corrupt stored value (wrong version byte) fails closed on read.
    let corrupt_key = kv_key(&col, 0, b"corrupt");
    vault
        .write_cf(
            ColumnFamily::Kv,
            corrupt_key,
            vec![0x02, 0, 0, 0, 0, 0, 0, 0, 0],
        )
        .unwrap();
    assert_eq!(
        layer.kv_get(&col, 0, b"corrupt").unwrap_err().code,
        "CALYX_ASTER_CORRUPT_SHARD"
    );

    // (5) truncated stored value (shorter than header) fails closed.
    let short_key = kv_key(&col, 0, b"short");
    vault
        .write_cf(ColumnFamily::Kv, short_key, vec![KV_VALUE_VERSION, 0, 0])
        .unwrap();
    assert_eq!(
        layer.kv_get(&col, 0, b"short").unwrap_err().code,
        "CALYX_ASTER_CORRUPT_SHARD"
    );
}

proptest! {
    #[test]
    fn set_then_get_roundtrips(
        ns in any::<u64>(),
        key in proptest::collection::vec(any::<u8>(), 1..64),
        val in proptest::collection::vec(any::<u8>(), 0..256),
    ) {
        let vault = AsterVault::with_clock(vault_id(), b"salt", FixedClock::new(1000));
        let layer = KvLayer::new(&vault);
        let col = kv_collection();
        layer.kv_set(&col, ns, &key, &val, None).unwrap();
        prop_assert_eq!(layer.kv_get(&col, ns, &key).unwrap(), Some(val));
    }
}

#[test]
fn durable_kv_fsv_writes_readback_artifacts() {
    let fsv_root = std::env::var_os("CALYX_FSV_ROOT").map(PathBuf::from);
    let dir = fsv_root
        .as_ref()
        .map(|root| root.join("kv-vault"))
        .unwrap_or_else(|| temp_dir("kv-fsv"));
    fs::remove_dir_all(&dir).ok();

    let vault = AsterVault::new_durable(
        &dir,
        vault_id(),
        b"kv-fsv-salt".to_vec(),
        VaultOptions::default(),
    )
    .unwrap();
    let layer = KvLayer::new(&vault);
    let col = kv_collection();
    create_collection(&vault, col.clone()).unwrap();

    let before = vault
        .scan_cf_at(vault.latest_seq(), ColumnFamily::Kv)
        .unwrap();
    assert!(before.is_empty());

    layer.kv_set(&col, 1, b"foo", b"bar", None).unwrap();
    let expected_key = kv_key(&col, 1, b"foo");
    let expected_value = encode_value(0, b"bar");
    let after = vault
        .scan_cf_at(vault.latest_seq(), ColumnFamily::Kv)
        .unwrap();
    assert_eq!(after, vec![(expected_key.clone(), expected_value.clone())]);

    vault.flush().unwrap();
    drop(vault);

    let reopened = AsterVault::open(
        &dir,
        vault_id(),
        b"kv-fsv-salt".to_vec(),
        VaultOptions::default(),
    )
    .unwrap();
    let reopened_layer = KvLayer::new(&reopened);
    let raw_after_reopen = reopened
        .read_cf_at(reopened.latest_seq(), ColumnFamily::Kv, &expected_key)
        .unwrap()
        .unwrap();
    assert_eq!(raw_after_reopen, expected_value);
    assert_eq!(raw_after_reopen[0], KV_VALUE_VERSION);
    assert_eq!(
        reopened_layer.kv_get(&col, 1, b"foo").unwrap(),
        Some(b"bar".to_vec())
    );

    let cf_files = physical_files(&dir.join("cf").join("kv"));
    assert!(!cf_files.is_empty(), "cf/kv must hold on-disk shards");

    let readback = serde_json::json!({
        "issue": 453,
        "layer": "kv",
        "source_of_truth": dir.display().to_string(),
        "cf": ColumnFamily::Kv.name(),
        "key_hex": hex_bytes(&expected_key),
        "key_discriminant": format!("{:#04x}", expected_key[0]),
        "value_hex": hex_bytes(&expected_value),
        "cold_open_equal": raw_after_reopen == expected_value,
        "decoded_after_reopen": String::from_utf8_lossy(
            &reopened_layer.kv_get(&col, 1, b"foo").unwrap().unwrap()
        ),
        "kv_cf_files": cf_files,
    });
    assert_eq!(readback["cold_open_equal"], serde_json::json!(true));

    if let Some(root) = fsv_root {
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("ph53-kv-readback.json"),
            serde_json::to_vec_pretty(&readback).unwrap(),
        )
        .unwrap();
        println!("ph53_kv_fsv_root={}", root.display());
        println!("{}", serde_json::to_string_pretty(&readback).unwrap());
    } else {
        fs::remove_dir_all(dir).ok();
    }
}

fn physical_files(dir: &std::path::Path) -> Vec<serde_json::Value> {
    let mut files = Vec::new();
    if dir.exists() {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            let bytes = fs::read(&path).unwrap();
            files.push(serde_json::json!({
                "path": path.display().to_string(),
                "bytes": bytes.len(),
            }));
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

fn temp_dir(name: &str) -> PathBuf {
    let id = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("calyx-aster-{name}-{}-{id}", std::process::id()));
    fs::remove_dir_all(&dir).ok();
    dir
}
