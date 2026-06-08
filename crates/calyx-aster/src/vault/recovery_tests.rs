use super::*;
use calyx_core::{AbsentReason, CxFlags, InputRef, LedgerRef, Modality, SlotVector, VaultStore};
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

#[test]
fn open_recovers_manifested_rows_from_ssts_when_wal_history_is_absent() {
    let dir = test_dir("manifested-sst-without-wal");
    let vault = AsterVault::new_durable(&dir, vault_id(), b"salt", VaultOptions::default())
        .expect("open durable");
    let cx = sample_constellation();
    let id = cx.cx_id;

    vault.put(cx.clone()).expect("durable put");
    vault.flush().expect("flush durable");
    fs::remove_file(dir.join("wal/00000000000000000000.wal")).expect("remove WAL history");

    let reopened =
        AsterVault::open(&dir, vault_id(), b"salt", VaultOptions::default()).expect("cold open");
    let got = reopened.get(id, reopened.snapshot()).unwrap();
    let mut expected = cx;
    expected.provenance = got.provenance.clone();

    assert_eq!(reopened.snapshot(), 1);
    assert_eq!(got.provenance.seq, 0);
    assert_ne!(got.provenance.hash, [0x51; 32]);
    assert_eq!(got, expected);
    cleanup(dir);
}

#[test]
fn durable_open_empty_dir_starts_at_zero() {
    let dir = test_dir("durable-empty");
    let vault = AsterVault::open(&dir, vault_id(), b"salt", VaultOptions::default())
        .expect("open empty durable");

    assert_eq!(vault.snapshot(), 0);
    cleanup(dir);
}

fn sample_constellation() -> Constellation {
    let mut slots = BTreeMap::new();
    slots.insert(
        SlotId::new(0),
        SlotVector::Dense {
            dim: 3,
            data: vec![0.25, 0.5, 1.0],
        },
    );
    slots.insert(
        SlotId::new(1),
        SlotVector::Absent {
            reason: AbsentReason::Deferred,
        },
    );
    Constellation {
        cx_id: CxId::from_bytes([0x31; 16]),
        vault_id: vault_id(),
        panel_version: 11,
        created_at: 1780831800,
        input_ref: InputRef {
            hash: [0x31; 32],
            pointer: Some("synthetic://manifested-sst-without-wal".to_string()),
            redacted: false,
        },
        modality: Modality::Text,
        slots,
        scalars: BTreeMap::new(),
        anchors: Vec::new(),
        provenance: LedgerRef {
            seq: 1,
            hash: [0x51; 32],
        },
        flags: CxFlags {
            ungrounded: true,
            ..CxFlags::default()
        },
    }
}

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().expect("valid ULID")
}

fn test_dir(name: &str) -> PathBuf {
    let id = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "calyx-aster-vault-recovery-{name}-{}-{id}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create test dir");
    dir
}

fn cleanup(dir: PathBuf) {
    fs::remove_dir_all(dir).expect("cleanup test dir");
}
