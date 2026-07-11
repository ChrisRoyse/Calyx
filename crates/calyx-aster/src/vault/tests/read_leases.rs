use super::*;
use crate::gc::GcRateLimit;
use calyx_core::CalyxError;
use std::fs;
use std::sync::Arc;

#[derive(Clone, Debug)]
struct MutableClock {
    now: Arc<AtomicU64>,
}

impl MutableClock {
    fn new(ts: u64) -> Self {
        Self {
            now: Arc::new(AtomicU64::new(ts)),
        }
    }

    fn set(&self, ts: u64) {
        self.now.store(ts, Ordering::Release);
    }
}

impl Clock for MutableClock {
    fn now(&self) -> u64 {
        self.now.load(Ordering::Acquire)
    }
}

#[test]
fn pinned_snapshot_cf_reads_honor_explicit_lease_bound() {
    let clock = MutableClock::new(1_000);
    let vault = AsterVault::with_clock(vault_id(), b"salt".to_vec(), clock.clone());
    vault
        .write_cf(ColumnFamily::Base, b"k".to_vec(), b"v".to_vec())
        .expect("write physical Base CF row");
    let snapshot = vault.pin_reader(Freshness::FreshDerived, 60_000);
    let lease_id = snapshot.lease().id();
    let before = vault
        .scan_cf_snapshot(snapshot, ColumnFamily::Base)
        .expect("scan Base CF before clock advance");

    clock.set(6_001);
    let after_old_internal_window = vault
        .read_cf_snapshot(snapshot, ColumnFamily::Base, b"k")
        .expect("explicit 60s snapshot remains live after old 5s window");

    clock.set(snapshot.lease().expires_at());
    let expired = vault
        .read_cf_snapshot(snapshot, ColumnFamily::Base, b"k")
        .expect_err("explicit snapshot expires at its own bound");

    println!(
        "ASTER_PINNED_SNAPSHOT_FSV {}",
        serde_json::json!({
            "source_of_truth": "Aster Base CF rows read through one explicit Snapshot lease",
            "lease_id": lease_id,
            "pinned_seq": snapshot.seq(),
            "issued_at": snapshot.lease().issued_at(),
            "expires_at": snapshot.lease().expires_at(),
            "before_base_rows": before.len(),
            "after_old_internal_window_value": after_old_internal_window.as_deref(),
            "expired_error_code": expired.code,
        })
    );
    assert_eq!(before, vec![(b"k".to_vec(), b"v".to_vec())]);
    assert_eq!(after_old_internal_window, Some(b"v".to_vec()));
    assert_eq!(expired.code, "CALYX_READER_LEASE_EXPIRED");
    assert!(
        !vault.release_reader(lease_id),
        "expired read should abort the registered lease"
    );
}

#[test]
fn seq_paged_scan_pins_versions_until_every_page_completes() {
    let clock = MutableClock::new(1_366);
    let vault = AsterVault::with_clock(vault_id(), b"salt".to_vec(), clock);
    let snapshot = vault
        .write_cf_batch([
            (ColumnFamily::Base, b"a".to_vec(), b"a-v1".to_vec()),
            (ColumnFamily::Base, b"b".to_vec(), b"b-v1".to_vec()),
            (ColumnFamily::Base, b"c".to_vec(), b"c-v1".to_vec()),
        ])
        .unwrap();
    assert_eq!(snapshot, 1);
    let rate_limit = GcRateLimit {
        max_ops_per_run: 100,
        min_interval_ms: 0,
    };
    let mut rows = Vec::new();
    let mut during_scan_gc = None;

    let scan_result: calyx_core::Result<()> = vault.scan_cf_pages_at(
        snapshot,
        ColumnFamily::Base,
        1,
        |page| -> calyx_core::Result<()> {
            rows.extend(page);
            if during_scan_gc.is_none() {
                let update_seq = vault.write_cf_batch([
                    (ColumnFamily::Base, b"a".to_vec(), b"a-v2".to_vec()),
                    (ColumnFamily::Base, b"b".to_vec(), b"b-v2".to_vec()),
                    (ColumnFamily::Base, b"c".to_vec(), b"c-v2".to_vec()),
                ])?;
                assert_eq!(update_seq, 2);
                during_scan_gc = Some(vault.snapshot_version_gc_once(rate_limit)?);
            }
            Ok(())
        },
    );
    let after_scan_gc = vault.snapshot_version_gc_once(rate_limit).unwrap();
    let visible_rows = rows
        .iter()
        .map(|(key, value)| {
            (
                String::from_utf8_lossy(key).into_owned(),
                String::from_utf8_lossy(value).into_owned(),
            )
        })
        .collect::<Vec<_>>();
    let readback = serde_json::json!({
        "snapshot": snapshot,
        "scan_error_code": scan_result.as_ref().err().map(|error: &CalyxError| error.code),
        "visible_rows": visible_rows,
        "during_scan_gc": during_scan_gc,
        "after_scan_gc": after_scan_gc,
    });
    if let Some(root) = std::env::var_os("CALYX_SEQ_READ_LEASE_FSV_ROOT") {
        let root = std::path::PathBuf::from(root);
        fs::create_dir_all(&root).unwrap();
        let path = root.join("seq-read-lease-gc-readback.json");
        fs::write(&path, serde_json::to_vec_pretty(&readback).unwrap()).unwrap();
        println!("CALYX_SEQ_READ_LEASE_FSV={}", path.display());
    }
    println!("CALYX_SEQ_READ_LEASE_READBACK={readback}");

    scan_result.unwrap();
    assert_eq!(
        visible_rows,
        vec![
            ("a".to_string(), "a-v1".to_string()),
            ("b".to_string(), "b-v1".to_string()),
            ("c".to_string(), "c-v1".to_string()),
        ]
    );
    let during_scan_gc = during_scan_gc.expect("first page must trigger GC");
    assert_eq!(during_scan_gc.safe_point_seq, snapshot);
    assert_eq!(during_scan_gc.versions_reclaimed, 0);
    assert_eq!(after_scan_gc.safe_point_seq, 2);
    assert_eq!(after_scan_gc.versions_reclaimed, 3);
}
