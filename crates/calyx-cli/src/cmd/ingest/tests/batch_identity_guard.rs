//! Issue #1923: the immutable batch snapshot identity must reject any change to
//! the batch bytes between the captured snapshot and the authoritative write,
//! with `CALYX_INGEST_BATCH_CHANGED` and zero Base/Ledger authority writes.
//!
//! Each test captures the identity from the original bytes, then mutates the file
//! on disk (simulating a TOCTOU race a concurrent process would cause between
//! validation and the streaming write) and drives the real streaming write with
//! the stale identity. DB-native CF readback proves before == after.

use super::*;
use crate::cmd::IngestOutput;
use crate::cmd::ingest::route::IngestGpuRoute;

/// Base + Ledger CF row counts read straight from the persisted vault — the
/// ingest source of truth.
fn base_and_ledger_counts(resolved: &ResolvedVault) -> (usize, usize) {
    let vault = open_vault(resolved).unwrap();
    let snapshot = vault.snapshot();
    let base = vault
        .scan_cf_at(snapshot, ColumnFamily::Base)
        .unwrap()
        .len();
    let ledger = vault
        .scan_cf_at(snapshot, ColumnFamily::Ledger)
        .unwrap()
        .len();
    (base, ledger)
}

fn assert_mutation_rejected_with_zero_writes(name: &str, original: &str, mutated: &str) {
    let (root, resolved) = test_vault_with_registered_dense_lens(name);
    let jsonl = resolved.path.join("batch.jsonl");
    fs::write(&jsonl, original).unwrap();
    let validation = validate_batch_file(&jsonl).unwrap();
    // Capture the immutable identity, then mutate the file behind it.
    let identity = BatchSnapshotIdentity::capture(&jsonl, validation.row_count).unwrap();
    fs::write(&jsonl, mutated).unwrap();

    let before = base_and_ledger_counts(&resolved);
    assert_eq!(before, (0, 0), "{name}: precondition is an empty vault");

    let err = ingest_validated_batch_streaming_with_output(
        &resolved,
        &jsonl,
        IngestOutput::Summary,
        &identity,
        IngestGpuRoute::cold_workers_allowed(),
        None,
        None,
    )
    .unwrap_err();

    let after = base_and_ledger_counts(&resolved);
    assert_eq!(
        err.code(),
        "CALYX_INGEST_BATCH_CHANGED",
        "{name}: expected CALYX_INGEST_BATCH_CHANGED, got {}: {}",
        err.code(),
        err.message()
    );
    assert_eq!(
        after,
        (0, 0),
        "{name}: a rejected batch must not write any Base/Ledger rows"
    );
    fs::remove_dir_all(root).ok();
}

#[test]
fn same_row_count_value_swap_is_rejected_with_zero_writes() {
    assert_mutation_rejected_with_zero_writes(
        "issue1923-value-swap",
        &format!(
            "{}\n{}\n",
            batch_line("issue1923 original first row"),
            batch_line("issue1923 original second row")
        ),
        &format!(
            "{}\n{}\n",
            batch_line("issue1923 swapped different first"),
            batch_line("issue1923 swapped different second")
        ),
    );
}

#[test]
fn same_byte_length_in_place_edit_is_rejected_with_zero_writes() {
    // Identical non-blank row count AND identical byte length; only the SHA-256
    // differs, proving the digest (not just the length) is the authority.
    assert_mutation_rejected_with_zero_writes(
        "issue1923-inplace-edit",
        &format!(
            "{}\n{}\n",
            batch_line("issue1923 aaa"),
            batch_line("issue1923 ccc")
        ),
        &format!(
            "{}\n{}\n",
            batch_line("issue1923 bbb"),
            batch_line("issue1923 ddd")
        ),
    );
}

#[test]
fn truncated_batch_is_rejected_with_zero_writes() {
    assert_mutation_rejected_with_zero_writes(
        "issue1923-truncate",
        &format!(
            "{}\n{}\n",
            batch_line("issue1923 truncate first"),
            batch_line("issue1923 truncate second")
        ),
        &format!("{}\n", batch_line("issue1923 truncate first")),
    );
}

#[test]
fn appended_batch_is_rejected_with_zero_writes() {
    assert_mutation_rejected_with_zero_writes(
        "issue1923-append",
        &format!(
            "{}\n{}\n",
            batch_line("issue1923 append first"),
            batch_line("issue1923 append second")
        ),
        &format!(
            "{}\n{}\n{}\n",
            batch_line("issue1923 append first"),
            batch_line("issue1923 append second"),
            batch_line("issue1923 append third")
        ),
    );
}

#[test]
fn unchanged_batch_still_ingests_and_writes_base_rows() {
    // Control: when the bytes are unchanged the snapshot verifies and the batch
    // ingests normally, so the guard is not spuriously rejecting valid batches.
    let (root, resolved) = test_vault_with_registered_dense_lens("issue1923-unchanged");
    let jsonl = resolved.path.join("batch.jsonl");
    fs::write(
        &jsonl,
        format!(
            "{}\n{}\n",
            batch_line("issue1923 unchanged alpha"),
            batch_line("issue1923 unchanged beta")
        ),
    )
    .unwrap();
    let validation = validate_batch_file(&jsonl).unwrap();
    let identity = BatchSnapshotIdentity::capture(&jsonl, validation.row_count).unwrap();

    ingest_validated_batch_streaming_with_output(
        &resolved,
        &jsonl,
        IngestOutput::Summary,
        &identity,
        IngestGpuRoute::cold_workers_allowed(),
        None,
        None,
    )
    .unwrap();

    let (base, _ledger) = base_and_ledger_counts(&resolved);
    assert_eq!(base, 2, "unchanged batch must persist both Base rows");
    fs::remove_dir_all(root).ok();
}
