//! Regression tests for issue #1132: rows whose only durable home is a
//! Router memtable-flush SST must never be silently invisible to
//! full-restore (`restore_mvcc_rows: true`) opens.

use super::*;
use calyx_core::{AbsentReason, CxFlags, InputRef, LedgerRef, Modality, SlotVector, VaultStore};
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

/// Planting a Router-class SST row with no commit-domain durable home must
/// fail the full-restore open closed, while a latest-only open (whose read
/// path serves router content) still sees the row.
#[test]
fn full_restore_open_fails_closed_on_router_only_rows() {
    let dir = test_dir("router-only-gate");
    let vault = AsterVault::new_durable(&dir, vault_id(), b"salt", VaultOptions::default())
        .expect("open durable");
    vault.put(sample_constellation(0x31)).expect("durable put");
    vault.flush().expect("flush durable");
    drop(vault);

    // Physically strand one row: only durable home is a router-flush SST.
    let mut router = CfRouter::open(&dir, 1024 * 1024).expect("open raw router");
    router
        .put(
            ColumnFamily::Graph,
            b"stranded-edge-key",
            b"stranded-edge-value",
        )
        .expect("raw router put");
    router.flush_cf(ColumnFamily::Graph).expect("router flush");
    drop(router);

    let error = AsterVault::open(&dir, vault_id(), b"salt", VaultOptions::default())
        .expect_err("full-restore open must fail closed on router-only rows");
    assert_eq!(error.code, "CALYX_ASTER_ROUTER_ONLY_ROWS");
    assert!(error.message.contains("graph"), "{}", error.message);
    assert!(error.message.contains("1 row(s)"), "{}", error.message);

    // The latest-only read path serves router content, so the same vault
    // opens and the stranded row is readable — the remediation is honest.
    let reader = AsterVault::open(
        &dir,
        vault_id(),
        b"salt",
        VaultOptions {
            restore_mvcc_rows: false,
            restore_ledger_hook: false,
            read_only: true,
            ..VaultOptions::default()
        },
    )
    .expect("latest-only open serves router content");
    let got = reader
        .read_cf_at(reader.snapshot(), ColumnFamily::Graph, b"stranded-edge-key")
        .expect("latest read")
        .expect("stranded row visible to latest-only reads");
    assert_eq!(got, b"stranded-edge-value");
    drop(reader);
    cleanup(dir);
}

/// A WAL-tail batch (committed, router-flushed, never checkpoint-flushed)
/// must be re-staged on reopen so a later flush writes its durable-batch
/// SSTs before the manifest advances past it. Deleting the WAL afterwards
/// proves the rows now live in the commit-domain durable view.
#[test]
fn wal_tail_batches_are_restaged_and_checkpointed_on_reopen() {
    let dir = test_dir("wal-tail-restage");
    let vault = AsterVault::new_durable(&dir, vault_id(), b"salt", VaultOptions::default())
        .expect("open durable");
    let cx1 = sample_constellation(0x31);
    vault.put(cx1.clone()).expect("put cx1");
    vault.flush().expect("flush covers cx1");

    // cx2 commits to WAL + router memtable; the router memtable is flushed
    // to a Router SST but the checkpoint flush never happens (crash model).
    let cx2 = sample_constellation(0x42);
    vault.put(cx2.clone()).expect("put cx2");
    vault.flush_all_cfs().expect("router memtable flush only");
    drop(vault);

    // Reopen (WAL tail restores cx2), write cx3, flush. Without re-staging,
    // this manifest advance would strand cx2 behind the WAL replay floor.
    let reopened = AsterVault::new_durable(&dir, vault_id(), b"salt", VaultOptions::default())
        .expect("reopen with WAL tail");
    let cx3 = sample_constellation(0x53);
    reopened.put(cx3.clone()).expect("put cx3");
    reopened.flush().expect("flush advances manifest");
    drop(reopened);

    // Remove ALL WAL history: surviving rows must come from durable SSTs.
    fs::remove_dir_all(dir.join("wal")).expect("drop WAL history");

    let cold = AsterVault::open(&dir, vault_id(), b"salt", VaultOptions::default())
        .expect("cold full-restore open passes the router coverage gate");
    for cx in [&cx1, &cx2, &cx3] {
        let got = cold
            .get(cx.cx_id, cold.snapshot())
            .expect("row visible after WAL removal");
        assert_eq!(got.cx_id, cx.cx_id);
    }
    drop(cold);
    cleanup(dir);
}

/// Tombstone purge on a handle that recovered a WAL tail must keep every row
/// visible to later full-restore opens: the compacted output has to be
/// covered by the manifest before the purge reclaims its inputs.
#[test]
fn purge_over_wal_tail_keeps_rows_visible_to_full_restore() {
    let dir = test_dir("purge-wal-tail");
    let vault = AsterVault::new_durable(&dir, vault_id(), b"salt", VaultOptions::default())
        .expect("open durable");
    let cx1 = sample_constellation(0x31);
    vault.put(cx1.clone()).expect("put cx1");
    vault.flush().expect("flush covers cx1");
    let cx2 = sample_constellation(0x42);
    vault.put(cx2.clone()).expect("put cx2");
    vault.flush_all_cfs().expect("router memtable flush only");
    drop(vault);

    // Reopen with the WAL tail and purge Base: pre-#1132 this named the
    // compacted output beyond the manifest floor and deleted the readable
    // inputs, silently erasing the CF from every full-restore open.
    let reopened = AsterVault::new_durable(&dir, vault_id(), b"salt", VaultOptions::default())
        .expect("reopen with WAL tail");
    reopened
        .purge_tombstoned_cfs(&[ColumnFamily::Base])
        .expect("purge under recovered WAL tail");
    drop(reopened);

    let manifest_durable_seq = crate::manifest::ManifestStore::open(&dir)
        .load_current()
        .expect("load manifest")
        .durable_seq;
    let base_dir = dir.join("cf/base");
    let compacted: Vec<PathBuf> = fs::read_dir(&base_dir)
        .expect("read base CF dir")
        .map(|entry| entry.expect("cf entry").path())
        .filter(|path| {
            matches!(
                crate::storage_names::classify_sst(path),
                Ok(Some(crate::storage_names::SstName::Compacted { .. }))
            )
        })
        .collect();
    for path in &compacted {
        let Ok(Some(crate::storage_names::SstName::Compacted { seq })) =
            crate::storage_names::classify_sst(path)
        else {
            unreachable!("filtered to compacted names");
        };
        assert!(
            seq <= manifest_durable_seq,
            "compacted output {} beyond manifest durable_seq {manifest_durable_seq}",
            path.display()
        );
    }

    let cold = AsterVault::open(&dir, vault_id(), b"salt", VaultOptions::default())
        .expect("cold full-restore open after purge");
    for cx in [&cx1, &cx2] {
        let got = cold
            .get(cx.cx_id, cold.snapshot())
            .expect("row visible after purge + cold open");
        assert_eq!(got.cx_id, cx.cx_id);
    }
    drop(cold);
    cleanup(dir);
}

fn sample_constellation(tag: u8) -> Constellation {
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
        cx_id: CxId::from_bytes([tag; 16]),
        vault_id: vault_id(),
        panel_version: 11,
        created_at: 1780831800,
        input_ref: InputRef {
            hash: [tag; 32],
            pointer: Some(format!("synthetic://issue1132-{tag:02x}")),
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
        "calyx-aster-issue1132-{name}-{}-{id}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create test dir");
    dir
}

fn cleanup(dir: PathBuf) {
    fs::remove_dir_all(dir).expect("cleanup test dir");
}
