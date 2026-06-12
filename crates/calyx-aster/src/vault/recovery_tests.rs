use super::*;
use calyx_core::{AbsentReason, CxFlags, InputRef, LedgerRef, Modality, SlotVector, VaultStore};
use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
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
    assert_eq!(vault.recovery_report().last_recovered_seq, 0);
    assert_eq!(vault.recovery_report().torn_tail, None);
    cleanup(dir);
}

#[test]
fn stale_durable_handle_refreshes_before_commit_sequence_allocation() {
    let dir = test_dir("stale-handle-commit-seq");
    let first = AsterVault::new_durable(&dir, vault_id(), b"salt", VaultOptions::default())
        .expect("open first durable");
    let stale = AsterVault::new_durable(&dir, vault_id(), b"salt", VaultOptions::default())
        .expect("open stale durable");
    let first_cx = sample_constellation();
    let mut second_cx = sample_constellation();
    second_cx.cx_id = CxId::from_bytes([0x42; 16]);
    second_cx.input_ref.hash = [0x42; 32];
    second_cx.input_ref.pointer = Some("synthetic://stale-handle-second".to_string());
    let first_id = first_cx.cx_id;
    let second_id = second_cx.cx_id;

    first.put(first_cx).expect("first handle put");
    first.flush().expect("first flush");
    stale
        .put(second_cx)
        .expect("stale handle put after external commit");
    stale.flush().expect("stale flush");

    let replay = crate::wal::replay_dir(dir.join("wal")).expect("replay stale handle wal");
    let reopened = AsterVault::open(&dir, vault_id(), b"salt", VaultOptions::default())
        .expect("reopen stale handle vault");

    assert_eq!(
        replay
            .records
            .iter()
            .map(|record| record.seq)
            .collect::<Vec<_>>(),
        vec![1, 2]
    );
    assert_eq!(stale.snapshot(), 2);
    assert_eq!(reopened.snapshot(), 2);
    assert_eq!(
        reopened.get(first_id, reopened.snapshot()).unwrap().cx_id,
        first_id
    );
    assert_eq!(
        reopened.get(second_id, reopened.snapshot()).unwrap().cx_id,
        second_id
    );
    cleanup(dir);
}

#[test]
fn open_reports_torn_tail_through_recovery_report() {
    let fsv_root = std::env::var_os("CALYX_FSV_ROOT").map(PathBuf::from);
    let dir = fsv_root.as_ref().map_or_else(
        || test_dir("open-torn-tail-report"),
        |root| {
            let dir = root.join("open-torn-tail-report").join("vault");
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).expect("create fsv vault");
            dir
        },
    );
    let vault = AsterVault::new_durable(&dir, vault_id(), b"salt", VaultOptions::default())
        .expect("open durable");
    let cx = sample_constellation();
    let id = cx.cx_id;
    vault.put(cx.clone()).expect("durable put");
    vault.flush().expect("flush durable");
    drop(vault);

    let wal_path = dir.join("wal/00000000000000000000.wal");
    let before_torn_bytes = fs::metadata(&wal_path).expect("wal metadata").len();
    let mut file = OpenOptions::new()
        .append(true)
        .open(&wal_path)
        .expect("open wal for torn append");
    file.write_all(b"CXW1partial").expect("write torn bytes");
    file.sync_data().expect("fsync torn bytes");
    drop(file);
    let after_torn_bytes = fs::metadata(&wal_path).expect("wal metadata").len();

    let reopened =
        AsterVault::open(&dir, vault_id(), b"salt", VaultOptions::default()).expect("cold open");
    let report = reopened.recovery_report();
    let tail = report.torn_tail.as_ref().expect("torn tail reported");
    let truncated_bytes = fs::metadata(&wal_path).expect("wal metadata").len();
    let got = reopened
        .get(id, reopened.snapshot())
        .expect("get recovered");

    assert_eq!(report.last_recovered_seq, 1);
    assert_eq!(tail.code, "CALYX_ASTER_TORN_WAL");
    assert_eq!(tail.offset, before_torn_bytes);
    assert_eq!(truncated_bytes, before_torn_bytes);
    assert!(after_torn_bytes > before_torn_bytes);
    assert_eq!(got.cx_id, id);
    if let Some(root) = fsv_root {
        let readback = serde_json::json!({
            "snapshot": reopened.snapshot(),
            "last_recovered_seq": report.last_recovered_seq,
            "torn_code": tail.code,
            "torn_offset": tail.offset,
            "before_torn_bytes": before_torn_bytes,
            "after_torn_bytes": after_torn_bytes,
            "truncated_bytes": truncated_bytes,
            "cx_id": id.to_string(),
        });
        fs::write(
            root.join("open-torn-tail-report-readback.json"),
            serde_json::to_vec_pretty(&readback).unwrap(),
        )
        .unwrap();
    } else {
        cleanup(dir);
    }
}

#[test]
fn cold_open_fails_closed_on_unrecognized_sst_name() {
    let dir = test_dir("unrecognized-sst-name");
    let vault = AsterVault::new_durable(&dir, vault_id(), b"salt", VaultOptions::default())
        .expect("open durable");
    vault.put(sample_constellation()).expect("durable put");
    vault.flush().expect("flush durable");
    drop(vault);
    fs::write(dir.join("cf/base/garbage.sst"), b"not an aster name").expect("plant stray sst");

    let error = AsterVault::open(&dir, vault_id(), b"salt", VaultOptions::default())
        .expect_err("cold open with unrecognized SST name");

    assert_eq!(error.code, "CALYX_ASTER_CORRUPT_SHARD");
    assert!(error.message.contains("garbage.sst"), "{}", error.message);
    cleanup(dir);
}

#[test]
fn cold_open_fails_closed_on_renamed_durable_sst() {
    let dir = test_dir("renamed-durable-sst");
    let vault = AsterVault::new_durable(&dir, vault_id(), b"salt", VaultOptions::default())
        .expect("open durable");
    vault.put(sample_constellation()).expect("durable put");
    vault.flush().expect("flush durable");
    drop(vault);
    let base_dir = dir.join("cf/base");
    let durable_sst = fs::read_dir(&base_dir)
        .expect("read base dir")
        .map(|entry| entry.expect("entry").path())
        .find(|path| {
            path.extension().and_then(|value| value.to_str()) == Some("sst")
                && path
                    .file_stem()
                    .and_then(|value| value.to_str())
                    .is_some_and(|stem| stem.contains('-'))
        })
        .expect("durable batch SST present");
    // Simulate name corruption / partial rename of a real durable batch file.
    fs::rename(&durable_sst, base_dir.join("0000007-x.sst")).expect("corrupt sst name");

    let error = AsterVault::open(&dir, vault_id(), b"salt", VaultOptions::default())
        .expect_err("cold open with corrupted SST name");

    assert_eq!(error.code, "CALYX_ASTER_CORRUPT_SHARD");
    assert!(error.message.contains("0000007-x.sst"), "{}", error.message);
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
        metadata: BTreeMap::new(),
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
