//! Regression coverage for issue #1307: failures after WAL durability must
//! never be reported as successful commits or hidden on stderr.

use super::super::*;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

const POST_WAL_ERROR: &str = "CALYX_DURABLE_COMMIT_RECONCILIATION_REQUIRED";
static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

fn fail_next_mvcc_commit(vault: &AsterVault) {
    vault
        .durable
        .as_ref()
        .expect("test MVCC failpoint requires durable vault")
        .fail_next_mvcc_commit();
}

fn fail_next_mvcc_restore(vault: &AsterVault) {
    vault
        .durable
        .as_ref()
        .expect("test MVCC restore failpoint requires durable vault")
        .fail_next_mvcc_restore();
}

fn fail_next_checkpoint(vault: &AsterVault) {
    vault
        .durable
        .as_ref()
        .expect("test checkpoint failpoint requires durable vault")
        .fail_next_checkpoint();
}

#[test]
fn post_wal_mvcc_failure_is_caller_visible_after_successful_reconciliation() {
    let dir = test_dir("reconciled");
    let vault = open_vault(&dir);
    fail_next_mvcc_commit(&vault);

    let error = vault
        .write_cf(
            ColumnFamily::Base,
            b"issue-1307-key".to_vec(),
            b"value".to_vec(),
        )
        .expect_err("post-WAL MVCC failure must not return Ok");
    assert_eq!(error.code, POST_WAL_ERROR);
    assert!(error.message.contains("wal_seq=1"), "{}", error.message);
    assert!(error.message.contains("restore=ok"), "{}", error.message);
    assert!(error.message.contains("checkpoint=ok"), "{}", error.message);
    assert_eq!(vault.latest_seq(), 1);
    assert_eq!(
        vault
            .read_cf_at(1, ColumnFamily::Base, b"issue-1307-key")
            .expect("read reconciled row"),
        Some(b"value".to_vec())
    );
    drop(vault);

    let manifest = crate::manifest::ManifestStore::open(&dir)
        .load_current()
        .expect("checkpoint manifest");
    assert_eq!(manifest.durable_seq, 1);
    fs::remove_dir_all(dir.join("wal")).expect("remove WAL to prove checkpoint bytes");
    let cold = open_vault(&dir);
    assert_eq!(
        cold.read_cf_at(1, ColumnFamily::Base, b"issue-1307-key")
            .expect("read checkpoint-only row"),
        Some(b"value".to_vec())
    );
    drop(cold);
    cleanup(dir);
}

#[test]
fn post_wal_error_reports_restore_and_checkpoint_failures_and_wal_recovers() {
    let dir = test_dir("recovery-required");
    let vault = open_vault(&dir);
    fail_next_mvcc_commit(&vault);
    fail_next_mvcc_restore(&vault);
    fail_next_checkpoint(&vault);

    let error = vault
        .write_cf(
            ColumnFamily::Base,
            b"issue-1307-key".to_vec(),
            b"value".to_vec(),
        )
        .expect_err("post-WAL reconciliation failures must reach the caller");
    assert_eq!(error.code, POST_WAL_ERROR);
    assert!(error.message.contains("wal_seq=1"), "{}", error.message);
    assert!(
        error
            .message
            .contains("restore=error[CALYX_ASTER_CORRUPT_SHARD]"),
        "{}",
        error.message
    );
    assert!(
        error
            .message
            .contains("checkpoint=error[CALYX_DISK_PRESSURE]"),
        "{}",
        error.message
    );
    assert_eq!(vault.latest_seq(), 0);
    drop(vault);

    let cold = open_vault(&dir);
    assert_eq!(cold.latest_seq(), 1);
    assert_eq!(
        cold.read_cf_at(1, ColumnFamily::Base, b"issue-1307-key")
            .expect("WAL replay read"),
        Some(b"value".to_vec())
    );
    drop(cold);
    cleanup(dir);
}

fn open_vault(dir: &PathBuf) -> AsterVault {
    AsterVault::new_durable(dir, vault_id(), b"salt", VaultOptions::default())
        .expect("open durable vault")
}

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().expect("valid ULID")
}

fn test_dir(name: &str) -> PathBuf {
    let id = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "calyx-aster-issue1307-{name}-{}-{id}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create test dir");
    dir
}

fn cleanup(dir: PathBuf) {
    fs::remove_dir_all(dir).expect("cleanup test dir");
}
