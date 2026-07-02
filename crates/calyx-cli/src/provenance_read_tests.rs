//! Unit tests for [`crate::provenance_read`] (issue #1096).
//!
//! Fixtures reproduce the on-disk shape that broke the old near-seq
//! heuristic: durable-batch SSTs whose file seq (WAL commit seq) does not
//! match the provenance (ledger) seqs of the rows inside them.

use std::fs;
use std::path::PathBuf;

use calyx_aster::cf::{ColumnFamily, ledger_key};
use calyx_aster::sst::write_sst;
use calyx_core::SlotId;

use crate::provenance_read::{RowSource, VaultReadContext};

fn temp_root(name: &str) -> PathBuf {
    let mut path = std::env::temp_dir();
    path.push(format!(
        "calyx-provenance-read-{name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    path
}

fn cf_dir(root: &PathBuf, cf: &ColumnFamily) -> PathBuf {
    let dir = root.join("cf").join(cf.name());
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn slot_cf() -> ColumnFamily {
    ColumnFamily::slot(SlotId::new(8))
}

/// The issue #1096 shape: a group-committed batch persisted rows with
/// provenance seqs 100..=102 into a durable-batch SST named by commit seq 5.
/// The ledger SST of the same batch carries the provenance range, so the
/// reader must resolve commit seq 5 and read the rows from their own batch.
#[test]
fn group_committed_rows_resolve_through_ledger_commit_batch() {
    let root = temp_root("group-committed");
    let ledger = cf_dir(&root, &ColumnFamily::Ledger);
    write_sst(
        ledger.join("00000000000000000005-0000.sst"),
        [
            (ledger_key(100).as_slice(), b"l100".as_slice()),
            (ledger_key(101).as_slice(), b"l101"),
            (ledger_key(102).as_slice(), b"l102"),
        ],
    )
    .unwrap();
    let slot = cf_dir(&root, &slot_cf());
    write_sst(
        slot.join("00000000000000000005-0003.sst"),
        [
            (b"k1".as_slice(), b"v1".as_slice()),
            (b"k2", b"v2"),
            (b"k3", b"v3"),
        ],
    )
    .unwrap();

    let mut context = VaultReadContext::new(&root);
    let batch = context
        .latest_cf_rows_for_provenance(
            slot_cf(),
            &[
                (b"k1".to_vec(), 100),
                (b"k2".to_vec(), 101),
                (b"k3".to_vec(), 102),
            ],
        )
        .unwrap();

    for (key, value) in [(b"k1", b"v1"), (b"k2", b"v2"), (b"k3", b"v3")] {
        let row = batch.rows.get(key.as_slice()).unwrap().as_ref().unwrap();
        assert_eq!(row.value, value.to_vec());
        assert_eq!(row.source, RowSource::CommitBatch, "{}", String::from_utf8_lossy(key));
    }
    assert_eq!(batch.stats.commit_batch_rows, 3);
    assert_eq!(batch.stats.full_set_rows, 0);
    assert_eq!(batch.stats.unresolved_rows, 0);
    fs::remove_dir_all(root).ok();
}

/// Provenance seqs with no covering durable-batch ledger SST must still read
/// correctly through the metadata-pruned full level (the semantic source of
/// truth), never silently return `None`.
#[test]
fn uncovered_provenance_seq_reads_through_full_level() {
    let root = temp_root("full-level");
    let slot = cf_dir(&root, &slot_cf());
    write_sst(
        slot.join("00000000000000000005-0003.sst"),
        [(b"k1".as_slice(), b"old".as_slice())],
    )
    .unwrap();
    write_sst(
        slot.join("00000000000000000009-0002.sst"),
        [(b"k1".as_slice(), b"new".as_slice())],
    )
    .unwrap();

    let mut context = VaultReadContext::new(&root);
    let batch = context
        .latest_cf_rows_for_provenance(slot_cf(), &[(b"k1".to_vec(), 4242)])
        .unwrap();

    let row = batch.rows.get(b"k1".as_slice()).unwrap().as_ref().unwrap();
    // Newest-first: the later durable batch wins.
    assert_eq!(row.value, b"new".to_vec());
    assert_eq!(row.source, RowSource::FullSet);
    assert_eq!(batch.stats.full_set_rows, 1);
    fs::remove_dir_all(root).ok();
}

/// Router-flush ledger SSTs span many commit batches; their file seq is a
/// flush seq, not the commit seq of the contained rows. They must never be
/// used for commit-batch mapping.
#[test]
fn router_ledger_files_are_not_used_as_commit_index() {
    let root = temp_root("router-ledger");
    let ledger = cf_dir(&root, &ColumnFamily::Ledger);
    // Router flush named seq 7 containing provenance 100: a naive index would
    // map provenance 100 -> commit seq 7, where no slot row exists.
    write_sst(
        ledger.join("00000000000000000007.sst"),
        [(ledger_key(100).as_slice(), b"l100".as_slice())],
    )
    .unwrap();
    let slot = cf_dir(&root, &slot_cf());
    write_sst(
        slot.join("00000000000000000005-0003.sst"),
        [(b"k1".as_slice(), b"v1".as_slice())],
    )
    .unwrap();

    let mut context = VaultReadContext::new(&root);
    let batch = context
        .latest_cf_rows_for_provenance(slot_cf(), &[(b"k1".to_vec(), 100)])
        .unwrap();

    let row = batch.rows.get(b"k1".as_slice()).unwrap().as_ref().unwrap();
    assert_eq!(row.value, b"v1".to_vec());
    assert_eq!(row.source, RowSource::FullSet);
    fs::remove_dir_all(root).ok();
}

/// A CF with compacted files has rewritten row history; the commit-batch
/// shortcut must be bypassed so the newest-first full level (which includes
/// the compacted output) stays authoritative.
#[test]
fn compacted_cf_bypasses_commit_batch_shortcut() {
    let root = temp_root("compacted-bypass");
    let ledger = cf_dir(&root, &ColumnFamily::Ledger);
    write_sst(
        ledger.join("00000000000000000005-0000.sst"),
        [(ledger_key(100).as_slice(), b"l100".as_slice())],
    )
    .unwrap();
    let slot = cf_dir(&root, &slot_cf());
    write_sst(
        slot.join("00000000000000000005-0003.sst"),
        [(b"k1".as_slice(), b"stale".as_slice())],
    )
    .unwrap();
    write_sst(
        slot.join("compacted-00000000000000000009.sst"),
        [(b"k1".as_slice(), b"compacted".as_slice())],
    )
    .unwrap();

    let mut context = VaultReadContext::new(&root);
    let batch = context
        .latest_cf_rows_for_provenance(slot_cf(), &[(b"k1".to_vec(), 100)])
        .unwrap();

    let row = batch.rows.get(b"k1".as_slice()).unwrap().as_ref().unwrap();
    assert_eq!(row.value, b"compacted".to_vec());
    assert_eq!(row.source, RowSource::FullSet);
    fs::remove_dir_all(root).ok();
}

/// Edge cases: empty key set, provenance seq 0, u64::MAX, and a key that no
/// stage can resolve (reported as `None`, counted as unresolved — the caller
/// owns the fail-closed decision).
#[test]
fn boundary_seqs_and_unresolvable_keys() {
    let root = temp_root("boundaries");
    let ledger = cf_dir(&root, &ColumnFamily::Ledger);
    write_sst(
        ledger.join("00000000000000000001-0000.sst"),
        [(ledger_key(0).as_slice(), b"l0".as_slice())],
    )
    .unwrap();
    write_sst(
        ledger.join("00000000000000000002-0000.sst"),
        [(ledger_key(u64::MAX).as_slice(), b"lmax".as_slice())],
    )
    .unwrap();
    let slot = cf_dir(&root, &slot_cf());
    write_sst(
        slot.join("00000000000000000001-0001.sst"),
        [(b"k0".as_slice(), b"v0".as_slice())],
    )
    .unwrap();
    write_sst(
        slot.join("00000000000000000002-0001.sst"),
        [(b"kmax".as_slice(), b"vmax".as_slice())],
    )
    .unwrap();

    let mut context = VaultReadContext::new(&root);

    let empty = context
        .latest_cf_rows_for_provenance(slot_cf(), &[])
        .unwrap();
    assert!(empty.rows.is_empty());
    assert_eq!(empty.stats, Default::default());

    let batch = context
        .latest_cf_rows_for_provenance(
            slot_cf(),
            &[
                (b"k0".to_vec(), 0),
                (b"kmax".to_vec(), u64::MAX),
                (b"kghost".to_vec(), 17),
            ],
        )
        .unwrap();
    assert_eq!(
        batch.rows.get(b"k0".as_slice()).unwrap().as_ref().unwrap().value,
        b"v0".to_vec()
    );
    assert_eq!(
        batch
            .rows
            .get(b"kmax".as_slice())
            .unwrap()
            .as_ref()
            .unwrap()
            .value,
        b"vmax".to_vec()
    );
    assert_eq!(batch.rows.get(b"kghost".as_slice()).unwrap(), &None);
    assert_eq!(batch.stats.commit_batch_rows, 2);
    assert_eq!(batch.stats.unresolved_rows, 1);
    fs::remove_dir_all(root).ok();
}

/// A ledger SST with a non-canonical (non-8-byte) key means the ledger CF
/// cannot serve as a provenance index; the read must error loudly instead of
/// quietly degrading.
#[test]
fn malformed_ledger_key_fails_loud() {
    let root = temp_root("malformed-ledger");
    let ledger = cf_dir(&root, &ColumnFamily::Ledger);
    write_sst(
        ledger.join("00000000000000000005-0000.sst"),
        [(b"not-a-seq".as_slice(), b"x".as_slice())],
    )
    .unwrap();
    let slot = cf_dir(&root, &slot_cf());
    write_sst(
        slot.join("00000000000000000005-0001.sst"),
        [(b"k1".as_slice(), b"v1".as_slice())],
    )
    .unwrap();

    let mut context = VaultReadContext::new(&root);
    let error = context
        .latest_cf_rows_for_provenance(slot_cf(), &[(b"k1".to_vec(), 100)])
        .unwrap_err();
    assert!(
        error.contains("non-canonical ledger key"),
        "unexpected error: {error}"
    );
    fs::remove_dir_all(root).ok();
}
