use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use calyx_core::{SlotId, VaultId};
use proptest::prelude::*;
use serde_json::json;

use super::{CALYX_TXN_COST_CAP, CALYX_TXN_TIMEOUT, IsolationLevel, TxnHandle, TxnState};
use crate::cf::{ColumnFamily, slot_key};
use crate::layers::blob::{self, BlobId};
use crate::layers::document::DocId;
use crate::layers::kv;
use crate::layers::relational::{RecordKey, record_key};
use crate::layers::timeseries::{self, RollupWindow};
use crate::layers::{DocumentLayer, KvLayer, RelationalLayer, TimeSeriesLayer};

mod support;
use support::{
    Collections, constellation, durable_vault, fsv_evidence, memory_vault, order_row, temp_root,
    write_fsv,
};

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap()
}

#[test]
fn commit_spans_record_kv_and_slot_at_one_seq() {
    let vault = memory_vault();
    let cols = Collections::create(&vault, "same_seq");
    let handle = TxnHandle::new(vault.vault_id());
    let pk = RecordKey::from_u64(7);
    let cx = constellation(vault.vault_id(), 7);
    let rel_key = record_key(&cols.orders, &pk).unwrap();
    let kv_key = kv::kv_key(&cols.cache, 1, b"session");
    let slot_key = slot_key(cx.cx_id);

    let mut txn = handle
        .begin_on(
            &vault,
            IsolationLevel::Serializable,
            Some(500),
            Duration::from_millis(50),
        )
        .unwrap();
    txn.put_record(&vault, &cols.orders, &pk, &order_row("bolt", 2))
        .unwrap();
    txn.kv_set(&vault, &cols.cache, 1, b"session", b"active", None)
        .unwrap();
    txn.put_constellation(&vault, &cx).unwrap();
    let seq = txn.commit(&vault).unwrap();

    assert_eq!(
        vault
            .seq_for_key_at(seq, ColumnFamily::Relational, &rel_key)
            .unwrap(),
        Some(seq)
    );
    assert_eq!(
        vault
            .seq_for_key_at(seq, ColumnFamily::Kv, &kv_key)
            .unwrap(),
        Some(seq)
    );
    assert_eq!(
        vault
            .seq_for_key_at(seq, ColumnFamily::slot(SlotId::new(0)), &slot_key)
            .unwrap(),
        Some(seq)
    );
    assert_eq!(
        KvLayer::new(&vault)
            .kv_get(&cols.cache, 1, b"session")
            .unwrap()
            .unwrap(),
        b"active"
    );
}

#[test]
fn serialization_timeout_then_retry_succeeds() {
    let vault = Arc::new(memory_vault());
    let handle = TxnHandle::new(vault.vault_id());
    let held = handle
        .begin_on(
            &vault,
            IsolationLevel::Serializable,
            Some(100),
            Duration::from_millis(50),
        )
        .unwrap();
    let h2 = handle.clone();
    let v2 = Arc::clone(&vault);
    let code = thread::spawn(move || {
        match h2.begin_on(
            &v2,
            IsolationLevel::Serializable,
            Some(100),
            Duration::from_millis(10),
        ) {
            Ok(_) => panic!("second txn unexpectedly acquired active handle"),
            Err(error) => error.code,
        }
    })
    .join()
    .unwrap();
    assert_eq!(code, CALYX_TXN_TIMEOUT);
    held.rollback().unwrap();
    assert!(
        handle
            .begin_on(
                &vault,
                IsolationLevel::Serializable,
                Some(100),
                Duration::from_millis(50)
            )
            .is_ok()
    );
}

#[test]
fn cost_cap_and_rollback_leave_vault_unchanged() {
    let vault = memory_vault();
    let cols = Collections::create(&vault, "cost_cap");
    let handle = TxnHandle::new(vault.vault_id());
    let pk = RecordKey::from_u64(99);
    let mut txn = handle
        .begin_on(
            &vault,
            IsolationLevel::Serializable,
            Some(1),
            Duration::from_millis(50),
        )
        .unwrap();
    txn.put_record(&vault, &cols.orders, &pk, &order_row("late", 1))
        .unwrap();
    thread::sleep(Duration::from_millis(5));
    let err = txn.commit(&vault).unwrap_err();
    assert_eq!(err.code, CALYX_TXN_COST_CAP);
    assert!(
        RelationalLayer::new(&vault)
            .get_record(&cols.orders, &pk)
            .unwrap()
            .is_none()
    );
    assert!(matches!(handle.state().unwrap(), TxnState::Idle));
}

#[test]
fn rollback_drop_overlay_and_empty_commit_edges() {
    let vault = memory_vault();
    let cols = Collections::create(&vault, "edges");
    let handle = TxnHandle::new(vault.vault_id());

    let pk = RecordKey::from_u64(1);
    let mut txn = handle
        .begin_on(
            &vault,
            IsolationLevel::Serializable,
            Some(100),
            Duration::from_millis(50),
        )
        .unwrap();
    txn.put_record(&vault, &cols.orders, &pk, &order_row("rollback", 1))
        .unwrap();
    txn.rollback().unwrap();
    assert!(
        RelationalLayer::new(&vault)
            .get_record(&cols.orders, &pk)
            .unwrap()
            .is_none()
    );

    {
        let mut dropped = handle
            .begin_on(
                &vault,
                IsolationLevel::Serializable,
                Some(100),
                Duration::from_millis(50),
            )
            .unwrap();
        dropped
            .put_record(
                &vault,
                &cols.orders,
                &RecordKey::from_u64(2),
                &order_row("drop", 2),
            )
            .unwrap();
    }
    assert!(
        RelationalLayer::new(&vault)
            .get_record(&cols.orders, &RecordKey::from_u64(2))
            .unwrap()
            .is_none()
    );

    let mut serial = handle
        .begin_on(
            &vault,
            IsolationLevel::Serializable,
            Some(100),
            Duration::from_millis(50),
        )
        .unwrap();
    let pk3 = RecordKey::from_u64(3);
    serial
        .put_record(&vault, &cols.orders, &pk3, &order_row("overlay", 3))
        .unwrap();
    assert!(
        serial
            .get_record(&vault, &cols.orders, &pk3)
            .unwrap()
            .is_some()
    );
    assert!(
        RelationalLayer::new(&vault)
            .get_record(&cols.orders, &pk3)
            .unwrap()
            .is_none()
    );
    serial.rollback().unwrap();

    let mut rc = handle
        .begin_on(
            &vault,
            IsolationLevel::ReadCommitted,
            Some(100),
            Duration::from_millis(50),
        )
        .unwrap();
    rc.put_record(&vault, &cols.orders, &pk3, &order_row("rc", 4))
        .unwrap();
    assert!(rc.get_record(&vault, &cols.orders, &pk3).unwrap().is_none());
    rc.rollback().unwrap();

    let before = vault.latest_seq();
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
    assert!(seq > before);
    assert_eq!(
        vault.scan_cf_at(seq, ColumnFamily::Online).unwrap().len(),
        1
    );
}

#[test]
fn read_committed_staging_keeps_rollups_consistent() {
    let vault = memory_vault();
    let cols = Collections::create(&vault, "read_committed_rollup");
    let handle = TxnHandle::new(vault.vault_id());
    let series = 9;
    let first_ts = 1_000;
    let rollup_key = timeseries::rollup_key(
        &cols.metrics,
        series,
        RollupWindow::OneMinute,
        RollupWindow::OneMinute.window_start(first_ts),
    );

    let mut txn = handle
        .begin_on(
            &vault,
            IsolationLevel::ReadCommitted,
            Some(500),
            Duration::from_millis(50),
        )
        .unwrap();
    txn.ts_write(&vault, &cols.metrics, series, first_ts, 2.0)
        .unwrap();
    txn.ts_write(&vault, &cols.metrics, series, first_ts + 1, 3.5)
        .unwrap();
    assert!(
        txn.read_cf(&vault, ColumnFamily::TimeSeries, &rollup_key)
            .unwrap()
            .is_none()
    );

    let seq = txn.commit(&vault).unwrap();
    let bytes = vault
        .read_cf_at(seq, ColumnFamily::TimeSeries, &rollup_key)
        .unwrap()
        .unwrap();
    let rollup = timeseries::decode_rollup(&bytes).unwrap();
    assert_eq!(rollup.count, 2);
    assert_eq!(rollup.sum, 5.5);
    assert_eq!(rollup.min, 2.0);
    assert_eq!(rollup.max, 3.5);
    assert_eq!(
        TimeSeriesLayer::new(&vault)
            .ts_range(&cols.metrics, series, 0, u64::MAX)
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn wal_submit_failure_is_fail_closed_and_releases_handle() {
    let root = temp_root("issue463-wal-fail");
    let vault = durable_vault(&root);
    let cols = Collections::create(&vault, "wal_fail");
    let handle = TxnHandle::new(vault.vault_id());
    let pk = RecordKey::from_u64(5);
    let mut txn = handle
        .begin_on(
            &vault,
            IsolationLevel::Serializable,
            Some(100),
            Duration::from_millis(50),
        )
        .unwrap();
    txn.put_record(&vault, &cols.orders, &pk, &order_row("wal", 5))
        .unwrap();
    vault.fail_next_wal_append_for_test();
    let err = txn.commit(&vault).unwrap_err();
    assert_eq!(err.code, "CALYX_DISK_PRESSURE");
    assert!(
        RelationalLayer::new(&vault)
            .get_record(&cols.orders, &pk)
            .unwrap()
            .is_none()
    );
    assert!(
        handle
            .begin_on(
                &vault,
                IsolationLevel::Serializable,
                Some(100),
                Duration::from_millis(50)
            )
            .is_ok()
    );
}

proptest! {
    #[test]
    fn sequential_txns_touch_all_plain_modes_without_seq_gaps(n in 1usize..5) {
        let vault = memory_vault();
        let cols = Collections::create(&vault, "prop");
        let handle = TxnHandle::new(vault.vault_id());
        let mut last = vault.latest_seq();
        for idx in 0..n {
            let doc_name = format!("doc-{idx}");
            let kv_name = format!("k{idx}");
            let blob_name = format!("b{idx}");
            let mut txn = handle.begin_on(
                &vault,
                IsolationLevel::Serializable,
                Some(500),
                Duration::from_millis(50),
            ).unwrap();
            let pk = RecordKey::from_u64(idx as u64 + 10);
            txn.put_record(&vault, &cols.orders, &pk, &order_row("prop", idx as i64)).unwrap();
            txn.put_doc(&vault, &cols.docs, DocId::from_text(&doc_name), &json!({"i": idx})).unwrap();
            txn.kv_set(&vault, &cols.cache, 1, kv_name.as_bytes(), b"v", None).unwrap();
            txn.ts_write(&vault, &cols.metrics, 1, 1_000 + idx as u64, idx as f64 + 1.0).unwrap();
            txn.blob_put_chunk(&vault, &cols.assets, BlobId::from_text(&blob_name), 0, b"chunk").unwrap();
            let seq = txn.commit(&vault).unwrap();
            prop_assert_eq!(seq, last + 1);
            prop_assert!(RelationalLayer::new(&vault).get_record(&cols.orders, &pk).unwrap().is_some());
            prop_assert!(DocumentLayer::new(&vault).get_doc(&cols.docs, DocId::from_text(&doc_name)).unwrap().is_some());
            prop_assert!(KvLayer::new(&vault).kv_get(&cols.cache, 1, kv_name.as_bytes()).unwrap().is_some());
            prop_assert_eq!(TimeSeriesLayer::new(&vault).ts_range(&cols.metrics, 1, 0, u64::MAX).unwrap().len(), idx + 1);
            let blob_key = blob::chunk_key(&cols.assets, BlobId::from_text(&blob_name), 0);
            prop_assert!(vault.read_cf_at(seq, ColumnFamily::Blob, &blob_key).unwrap().is_some());
            last = seq;
        }
    }
}

#[test]
fn issue463_fsv_local_readback_artifact() {
    let root = temp_root("issue463-fsv-local");
    let evidence = fsv_evidence(&root);
    write_fsv(&root, &evidence);
}

#[test]
#[ignore = "manual FSV writes transaction readback evidence bytes"]
fn issue463_fsv_manual_readback_artifact() {
    let root = PathBuf::from(
        std::env::var_os("CALYX_ISSUE463_FSV_ROOT")
            .expect("CALYX_ISSUE463_FSV_ROOT must point at a fresh evidence root"),
    );
    if root.exists() {
        panic!("CALYX_ISSUE463_FSV_ROOT must be fresh: {}", root.display());
    }
    fs::create_dir_all(&root).unwrap();
    let evidence = fsv_evidence(&root);
    write_fsv(&root, &evidence);
}
