use std::path::Path;
use std::thread;
use std::time::Duration;

use calyx_core::FixedClock;
use serde_json::{Value, json};

use super::{Collections, durable_vault, order_row, physical_files, wal_batches};
use crate::cf::ColumnFamily;
use crate::layers::relational::{RecordKey, record_key};
use crate::txn::{IsolationLevel, TxnHandle};
use crate::vault::AsterVault;

pub(super) fn edge_evidence(root: &Path) -> Value {
    json!({
        "cost_cap_exceeded": edge_cost_cap(root),
        "explicit_rollback": edge_rollback(root),
        "drop_rollback": edge_drop(root),
        "empty_commit": edge_empty_commit(root),
        "serializable_overlay": edge_overlay(root),
        "wal_submit_failure": edge_wal_failure(root)
    })
}

fn edge_cost_cap(root: &Path) -> Value {
    let vault_dir = root.join("edge-cost-cap-vault");
    let vault = durable_vault(&vault_dir);
    let cols = Collections::create(&vault, "edge_cost");
    let handle = TxnHandle::new(vault.vault_id());
    let pk = RecordKey::from_u64(9001);
    let rel_key = record_key(&cols.orders, &pk).unwrap();
    let before = read_record_state(&vault, vault.latest_seq(), &rel_key);
    let mut txn = handle
        .begin_on(
            &vault,
            IsolationLevel::Serializable,
            Some(1),
            Duration::from_millis(50),
        )
        .unwrap();
    txn.put_record(&vault, &cols.orders, &pk, &order_row("too-slow", 1))
        .unwrap();
    thread::sleep(Duration::from_millis(5));
    let err = txn.commit(&vault).unwrap_err();
    let after = read_record_state(&vault, vault.latest_seq(), &rel_key);
    vault.flush().unwrap();
    json!({
        "trigger": "commit after elapsed time exceeds 1ms cap",
        "expected": "CALYX_TXN_COST_CAP and no relational row",
        "error_code": err.code,
        "before": before,
        "after": after,
        "cf_files": physical_files(&vault_dir.join("cf")),
        "wal_batches": wal_batches(&vault_dir.join("wal"))
    })
}

fn edge_rollback(root: &Path) -> Value {
    let vault_dir = root.join("edge-rollback-vault");
    let vault = durable_vault(&vault_dir);
    let cols = Collections::create(&vault, "edge_rollback");
    let handle = TxnHandle::new(vault.vault_id());
    let pk = RecordKey::from_u64(9002);
    let rel_key = record_key(&cols.orders, &pk).unwrap();
    let before = read_record_state(&vault, vault.latest_seq(), &rel_key);
    let mut txn = handle
        .begin_on(
            &vault,
            IsolationLevel::Serializable,
            Some(100),
            Duration::from_millis(50),
        )
        .unwrap();
    txn.put_record(&vault, &cols.orders, &pk, &order_row("rollback", 2))
        .unwrap();
    let staged_rows = txn.batch_len();
    txn.rollback().unwrap();
    let after = read_record_state(&vault, vault.latest_seq(), &rel_key);
    vault.flush().unwrap();
    json!({
        "trigger": "rollback after staging one relational row",
        "expected": "staged row discarded; no WAL txn row",
        "staged_rows": staged_rows,
        "before": before,
        "after": after,
        "cf_files": physical_files(&vault_dir.join("cf")),
        "wal_batches": wal_batches(&vault_dir.join("wal"))
    })
}

fn edge_drop(root: &Path) -> Value {
    let vault_dir = root.join("edge-drop-vault");
    let vault = durable_vault(&vault_dir);
    let cols = Collections::create(&vault, "edge_drop");
    let handle = TxnHandle::new(vault.vault_id());
    let pk = RecordKey::from_u64(9003);
    let rel_key = record_key(&cols.orders, &pk).unwrap();
    let before = read_record_state(&vault, vault.latest_seq(), &rel_key);
    let staged_rows;
    {
        let mut txn = handle
            .begin_on(
                &vault,
                IsolationLevel::Serializable,
                Some(100),
                Duration::from_millis(50),
            )
            .unwrap();
        txn.put_record(&vault, &cols.orders, &pk, &order_row("drop", 3))
            .unwrap();
        staged_rows = txn.batch_len();
    }
    let after = read_record_state(&vault, vault.latest_seq(), &rel_key);
    let reacquired = handle
        .begin_on(
            &vault,
            IsolationLevel::Serializable,
            Some(100),
            Duration::from_millis(50),
        )
        .is_ok();
    vault.flush().unwrap();
    json!({
        "trigger": "drop CrossModelTxn without commit or rollback",
        "expected": "implicit rollback; handle can be acquired again",
        "staged_rows": staged_rows,
        "reacquired_after_drop": reacquired,
        "before": before,
        "after": after,
        "cf_files": physical_files(&vault_dir.join("cf")),
        "wal_batches": wal_batches(&vault_dir.join("wal"))
    })
}

fn edge_empty_commit(root: &Path) -> Value {
    let vault_dir = root.join("edge-empty-vault");
    let vault = durable_vault(&vault_dir);
    Collections::create(&vault, "edge_empty");
    let handle = TxnHandle::new(vault.vault_id());
    let before = read_online_state(&vault, vault.latest_seq());
    let expected_seq = vault.latest_seq() + 1;
    let seq = handle
        .begin_on(
            &vault,
            IsolationLevel::Serializable,
            Some(100),
            Duration::from_millis(50),
        )
        .unwrap()
        .commit(&vault)
        .unwrap();
    assert_eq!(seq, expected_seq);
    let after = read_online_state(&vault, seq);
    vault.flush().unwrap();
    json!({
        "trigger": "commit transaction with no staged writes",
        "expected": "seq advances by one and Online CF receives empty-txn marker",
        "expected_seq": expected_seq,
        "actual_seq": seq,
        "before": before,
        "after": after,
        "cf_files": physical_files(&vault_dir.join("cf")),
        "wal_batches": wal_batches(&vault_dir.join("wal"))
    })
}

fn edge_overlay(root: &Path) -> Value {
    let vault_dir = root.join("edge-overlay-vault");
    let vault = durable_vault(&vault_dir);
    let cols = Collections::create(&vault, "edge_overlay");
    let handle = TxnHandle::new(vault.vault_id());
    let pk = RecordKey::from_u64(9004);
    let rel_key = record_key(&cols.orders, &pk).unwrap();
    let before = read_record_state(&vault, vault.latest_seq(), &rel_key);
    let mut txn = handle
        .begin_on(
            &vault,
            IsolationLevel::Serializable,
            Some(100),
            Duration::from_millis(50),
        )
        .unwrap();
    txn.put_record(&vault, &cols.orders, &pk, &order_row("overlay", 4))
        .unwrap();
    let overlay_present = txn.get_record(&vault, &cols.orders, &pk).unwrap().is_some();
    let durable_during = read_record_state(&vault, vault.latest_seq(), &rel_key);
    let seq = txn.commit(&vault).unwrap();
    let after = read_record_state(&vault, seq, &rel_key);
    vault.flush().unwrap();
    json!({
        "trigger": "Serializable read after staged write before commit",
        "expected": "txn overlay sees row while durable CF stays absent until commit",
        "overlay_present_before_commit": overlay_present,
        "before": before,
        "durable_during_txn": durable_during,
        "after": after,
        "cf_files": physical_files(&vault_dir.join("cf")),
        "wal_batches": wal_batches(&vault_dir.join("wal"))
    })
}

fn edge_wal_failure(root: &Path) -> Value {
    let vault_dir = root.join("edge-wal-failure-vault");
    let vault = durable_vault(&vault_dir);
    let cols = Collections::create(&vault, "edge_wal");
    let handle = TxnHandle::new(vault.vault_id());
    let pk = RecordKey::from_u64(9005);
    let rel_key = record_key(&cols.orders, &pk).unwrap();
    let before = read_record_state(&vault, vault.latest_seq(), &rel_key);
    let mut txn = handle
        .begin_on(
            &vault,
            IsolationLevel::Serializable,
            Some(100),
            Duration::from_millis(50),
        )
        .unwrap();
    txn.put_record(&vault, &cols.orders, &pk, &order_row("wal-fail", 5))
        .unwrap();
    vault.fail_next_wal_append_for_test();
    let err = txn.commit(&vault).unwrap_err();
    let after = read_record_state(&vault, vault.latest_seq(), &rel_key);
    let reacquired = handle
        .begin_on(
            &vault,
            IsolationLevel::Serializable,
            Some(100),
            Duration::from_millis(50),
        )
        .is_ok();
    vault.flush().unwrap();
    json!({
        "trigger": "inject next WAL append failure before commit",
        "expected": "exact disk-pressure code; no partial relational row; handle released",
        "error_code": err.code,
        "reacquired_after_error": reacquired,
        "before": before,
        "after": after,
        "cf_files": physical_files(&vault_dir.join("cf")),
        "wal_batches": wal_batches(&vault_dir.join("wal"))
    })
}

fn read_record_state(vault: &AsterVault<FixedClock>, seq: u64, rel_key: &[u8]) -> Value {
    json!({
        "seq": seq,
        "latest_seq": vault.latest_seq(),
        "relational_seq": vault.seq_for_key_at(seq, ColumnFamily::Relational, rel_key).unwrap(),
        "relational_present": vault.read_cf_at(seq, ColumnFamily::Relational, rel_key).unwrap().is_some()
    })
}

fn read_online_state(vault: &AsterVault<FixedClock>, seq: u64) -> Value {
    json!({
        "seq": seq,
        "latest_seq": vault.latest_seq(),
        "online_rows": vault.scan_cf_at(seq, ColumnFamily::Online).unwrap().len()
    })
}
