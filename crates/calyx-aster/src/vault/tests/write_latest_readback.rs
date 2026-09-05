//! Substrate contract for [`VaultOptions::write_latest_readback`] (issue #1862
//! shared vault-open root cause).
//!
//! Proves the write-capable latest-readback open profile serves reads from the
//! mmap-backed CF router WITHOUT materializing durable rows into the in-memory
//! MVCC table, while preserving physical readback and write/ledger-append
//! capability. This is the substrate the weave-loom / kernel-build /
//! kernel-answer commands adopt to keep their working set proportional to what
//! they touch instead of the whole physical vault.

use super::*;
use calyx_ledger::{ActorId, EntryKind, SubjectId};

/// Seeded (key, value) rows returned alongside the last committed seq.
type SeededRows = Vec<(Vec<u8>, Vec<u8>)>;

/// Seeds `count` durably-checkpointed content-slot rows and returns their
/// (key, value) pairs plus the last committed seq. The rows are flushed so they
/// exist as manifested durable-batch SSTs — exactly the physical state a
/// full-restore open would copy into the heap.
fn seed_checkpointed_rows(vault: &AsterVault, cf: ColumnFamily, count: usize) -> (SeededRows, u64) {
    let mut rows = Vec::with_capacity(count);
    for index in 0..count {
        let key = (index as u64).to_be_bytes().to_vec();
        // A non-trivial payload so full-restore materialization is a real,
        // measurable heap cost, not a rounding error.
        let value = vec![0xAB_u8; 4096];
        vault
            .write_cf_batch([(cf, key.clone(), value.clone())])
            .expect("commit content row");
        rows.push((key, value));
    }
    vault.flush().expect("flush durable-batch SSTs + manifest");
    let seq = vault.latest_seq();
    (rows, seq)
}

#[test]
fn write_latest_readback_serves_router_reads_without_mvcc_materialization() {
    let dir = test_dir("write-latest-readback");
    let salt = b"issue1862-write-latest-readback".to_vec();
    let cf = ColumnFamily::slot(SlotId::new(0));

    // Seed a physical vault with durably-checkpointed content-slot rows.
    let writer = AsterVault::new_durable(&dir, vault_id(), salt.clone(), VaultOptions::default())
        .expect("open durable writer");
    let (rows, seeded_seq) = seed_checkpointed_rows(&writer, cf, 8);
    drop(writer);

    // Baseline: the legacy full-restore open (restore_mvcc_rows: true) copies
    // every durable row into the in-memory MVCC store.
    let full = AsterVault::open(&dir, vault_id(), salt.clone(), VaultOptions::default())
        .expect("full-restore reopen");
    assert_eq!(full.latest_seq(), seeded_seq);
    for (key, value) in &rows {
        assert!(
            full.rows.has_any_version(cf, key),
            "full-restore open must materialize durable row {key:?} into the MVCC store"
        );
        assert_eq!(
            full.read_cf_at(full.latest_seq(), cf, key).expect("read"),
            Some(value.clone())
        );
    }
    drop(full);

    // Fix: the write-capable latest-readback open does NOT copy durable rows
    // into the MVCC store — the working set stays proportional to touched pages.
    let latest = AsterVault::open(
        &dir,
        vault_id(),
        salt.clone(),
        VaultOptions::write_latest_readback(),
    )
    .expect("write-latest-readback reopen");
    assert_eq!(latest.latest_seq(), seeded_seq);
    for (key, value) in &rows {
        assert!(
            !latest.rows.has_any_version(cf, key),
            "write_latest_readback must NOT materialize durable row {key:?} into the MVCC store"
        );
        // Physical readback is preserved: the router serves the row from the
        // mmap-backed SST at the latest snapshot.
        assert_eq!(
            latest
                .read_cf_at(latest.latest_seq(), cf, key)
                .expect("router readback"),
            Some(value.clone()),
            "latest read must still return the physical row bytes for {key:?}"
        );
    }

    // Write capability is intact: read-after-write on the same handle sees a
    // newly committed row through the router (memtable), then survives flush.
    let new_key = 10_000_u64.to_be_bytes().to_vec();
    let new_value = vec![0xCD_u8; 4096];
    latest
        .write_cf_batch([(cf, new_key.clone(), new_value.clone())])
        .expect("write-capable latest-readback handle accepts writes");
    assert_eq!(
        latest
            .read_cf_at(latest.latest_seq(), cf, &new_key)
            .expect("read-after-write"),
        Some(new_value.clone()),
        "read-after-write must see the just-committed row via the router"
    );
    latest.flush().expect("flush new row");

    // Ledger append is intact and physically persisted (the restore_ledger_hook
    // stays on for write-capable handles): the append commits and the entry is
    // readable from the Ledger CF on disk via the point-read path.
    let commit_seq_before = latest.latest_seq();
    let ledger_ref = latest
        .append_ledger_entry(
            EntryKind::Admin,
            SubjectId::Guard(b"issue1862-substrate".to_vec()),
            b"{\"event\":\"write_latest_readback_substrate\"}".to_vec(),
            ActorId::System,
        )
        .expect("ledger append on write-capable latest-readback handle");
    // The append is a real durable commit: the vault commit seq advances (the
    // ledger-chain seq in `ledger_ref` is a separate numbering domain).
    assert!(
        latest.latest_seq() > commit_seq_before,
        "ledger append must advance the vault commit seq (got {} <= {commit_seq_before})",
        latest.latest_seq()
    );
    latest.flush().expect("flush ledger append");
    drop(latest);

    let persisted = crate::ledger_view::read_ledger_seq(&dir, ledger_ref.seq)
        .expect("point-read appended ledger seq")
        .expect("appended ledger row must be physically present on disk");
    assert_eq!(persisted.seq, ledger_ref.seq);
    assert!(
        !persisted.bytes.is_empty(),
        "physically persisted ledger row must carry bytes"
    );

    // Cold full-restore reopen after the latest-readback writes still recovers
    // every content row and the appended ledger entry — no rows were stranded.
    let reopened = AsterVault::open(&dir, vault_id(), salt, VaultOptions::default())
        .expect("cold full-restore reopen");
    for (key, value) in &rows {
        assert_eq!(
            reopened
                .read_cf_at(reopened.latest_seq(), cf, key)
                .expect("read"),
            Some(value.clone())
        );
    }
    assert_eq!(
        reopened
            .read_cf_at(reopened.latest_seq(), cf, &new_key)
            .expect("read new row"),
        Some(new_value)
    );
    cleanup(dir);
}
