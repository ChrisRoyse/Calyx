#![cfg(target_os = "linux")]

//! PH58 T01 manual FSV driver for reader leases and snapshot-pin watchdog.

use calyx_aster::cf::ColumnFamily;
use calyx_aster::gc::{BoundedStalenessSnapshot, DEFAULT_MAX_PINNED_SEQ_GAP};
use calyx_aster::mvcc::Freshness;
use calyx_aster::resource::{ResourceStatus, VramBudgetStatus};
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{Clock, Ts, VaultId};
use serde_json::json;
use std::fs;
use std::path::Path;
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

mod fsv_support;
use fsv_support::{fsv_root_os, reset_dir};

#[derive(Clone, Debug)]
struct SharedClock {
    now: Arc<AtomicU64>,
}

impl SharedClock {
    fn new(now: Ts) -> Self {
        Self {
            now: Arc::new(AtomicU64::new(now)),
        }
    }

    fn set(&self, now: Ts) {
        self.now.store(now, Ordering::Relaxed);
    }
}

impl Clock for SharedClock {
    fn now(&self) -> Ts {
        self.now.load(Ordering::Relaxed)
    }
}

#[test]
#[ignore = "manual FSV writes PH58 reader-lease source-of-truth artifacts"]
fn ph58_reader_lease_watchdog_fsv() {
    let root = fsv_root_os("CALYX_FSV_ROOT", "calyx-ph58-reader-leases");
    reset_dir(&root);
    let vault_dir = root.join("vault");
    fs::create_dir_all(&vault_dir).expect("create vault dir");
    fs::write(
        root.join("cleanup-tag.txt"),
        b"issue481 PH58 reader lease synthetic FSV data\n",
    )
    .expect("write cleanup tag");
    let clock = SharedClock::new(10_000);
    let vault = AsterVault::new_durable_with_clock(
        &vault_dir,
        vault_id(),
        b"issue481-ph58-reader-leases".to_vec(),
        VaultOptions::default(),
        clock.clone(),
    )
    .expect("open durable vault");
    let key = b"ph58-key".to_vec();
    let initial_seq = vault
        .write_cf(ColumnFamily::Base, key.clone(), b"v1".to_vec())
        .expect("initial write");
    let before = status(&vault, &vault_dir);
    assert_eq!(before.pinned.active_leases, 0);
    assert_eq!(before.pinned.oldest_pinned_seq_gap, 0);

    let tick_reader = vault.pin_reader(Freshness::FreshDerived, 100);
    assert_eq!(tick_reader.lease().pinned_seq(), initial_seq);
    for version in 2u8..=5 {
        vault
            .write_cf(ColumnFamily::Base, key.clone(), vec![b'v', b'0' + version])
            .expect("versioned update");
    }
    let before_abort = status(&vault, &vault_dir);
    assert_eq!(before_abort.pinned.active_leases, 1);
    assert_eq!(before_abort.pinned.oldest_pinned_seq, Some(initial_seq));
    assert_eq!(before_abort.pinned.oldest_pinned_seq_gap, 4);
    vault.flush().expect("flush before disk readback");
    let sst_bytes_before_abort = sst_bytes(&vault_dir);
    fs::write(root.join("df-before.txt"), df_text(&vault_dir)).expect("write df before");

    clock.set(10_101);
    let tick = vault.snapshot_gc_tick(DEFAULT_MAX_PINNED_SEQ_GAP);
    assert_eq!(tick.aborted_readers, vec![tick_reader.lease().id()]);
    assert_eq!(tick.metrics.reader_lease_expired_total, 1);
    assert_eq!(tick.metrics.oldest_pinned_seq_gap, 0);
    let after_tick = status(&vault, &vault_dir);
    assert_eq!(after_tick.pinned.active_leases, 0);
    assert_eq!(after_tick.pinned.reader_lease_expired_total, 1);

    clock.set(20_000);
    let read_reader = vault.pin_reader(Freshness::FreshDerived, 5);
    clock.set(20_005);
    let read_error = vault
        .read_pinned_cf(read_reader, ColumnFamily::Base, &key)
        .expect_err("exact-boundary expired read fails closed");
    assert_eq!(read_error.code, "CALYX_READER_LEASE_EXPIRED");
    let after_read_error = status(&vault, &vault_dir);
    assert_eq!(after_read_error.pinned.active_leases, 0);
    assert_eq!(after_read_error.pinned.reader_lease_expired_total, 2);

    let checkpoint = BoundedStalenessSnapshot::at_checkpoint(initial_seq);
    let before_bounded = status(&vault, &vault_dir);
    assert_eq!(checkpoint.seq(), initial_seq);
    let after_bounded = status(&vault, &vault_dir);
    assert_eq!(
        before_bounded.pinned.active_leases,
        after_bounded.pinned.active_leases
    );
    assert_eq!(
        before_bounded.pinned.oldest_pinned_seq_gap,
        after_bounded.pinned.oldest_pinned_seq_gap
    );

    vault.flush().expect("flush after watchdog");
    let sst_bytes_after_abort = sst_bytes(&vault_dir);
    fs::write(root.join("df-after.txt"), df_text(&vault_dir)).expect("write df after");
    assert_eq!(sst_bytes_after_abort, sst_bytes_before_abort);
    let metrics = after_read_error.to_metrics_text("issue481");
    assert!(metrics.contains("calyx_reader_lease_expired_total{vault=\"issue481\"} 2"));
    assert!(metrics.contains("calyx_oldest_pinned_seq_gap{vault=\"issue481\"} 0"));
    fs::write(root.join("ph58-reader-leases.prom"), &metrics).expect("write metrics");

    let readback = json!({
        "input": {
            "initial_seq": initial_seq,
            "known_update_count": 4,
            "tick_reader_duration_ms": 100,
            "read_reader_duration_ms": 5
        },
        "expected": {
            "before_abort_gap": 4,
            "tick_aborted_readers": [tick_reader.lease().id()],
            "expired_total_after_tick": 1,
            "expired_total_after_read_error": 2,
            "final_gap": 0,
            "sst_bytes_flat_after_abort": true
        },
        "actual": {
            "before": before,
            "before_abort": before_abort,
            "tick": tick,
            "read_error_code": read_error.code,
            "after_read_error": after_read_error,
            "bounded_checkpoint_seq": checkpoint.seq(),
            "after_bounded": after_bounded,
            "sst_bytes_before_abort": sst_bytes_before_abort,
            "sst_bytes_after_abort": sst_bytes_after_abort,
            "metrics_file": "ph58-reader-leases.prom",
            "df_before_file": "df-before.txt",
            "df_after_file": "df-after.txt"
        }
    });
    fs::write(
        root.join("ph58-reader-leases-readback.json"),
        serde_json::to_vec_pretty(&readback).expect("encode readback"),
    )
    .expect("write readback");
    println!(
        "PH58 reader lease FSV: gap_before={} expired_total={} sst_bytes={}",
        before_abort.pinned.oldest_pinned_seq_gap,
        after_read_error.pinned.reader_lease_expired_total,
        sst_bytes_after_abort
    );
}

fn status(vault: &AsterVault<SharedClock>, vault_dir: &Path) -> ResourceStatus {
    vault
        .resource_status(vault_dir, vram())
        .expect("resource status")
}

fn vram() -> VramBudgetStatus {
    VramBudgetStatus {
        budget_bytes: 0,
        used_bytes: 0,
        probe_warning: None,
    }
}

fn sst_bytes(vault_dir: &Path) -> u64 {
    let mut bytes = 0;
    collect_sst_bytes(&vault_dir.join("cf"), &mut bytes);
    bytes
}

fn collect_sst_bytes(dir: &Path, bytes: &mut u64) {
    if !dir.is_dir() {
        return;
    }
    for entry in fs::read_dir(dir).expect("read cf dir") {
        let path = entry.expect("cf entry").path();
        if path.is_dir() {
            collect_sst_bytes(&path, bytes);
        } else if path.extension().and_then(|value| value.to_str()) == Some("sst") {
            *bytes += fs::metadata(path).expect("sst metadata").len();
        }
    }
}

fn df_text(path: &Path) -> String {
    let output = Command::new("df")
        .arg("-B1")
        .arg(path)
        .output()
        .expect("run df");
    assert!(
        output.status.success(),
        "df failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("df stdout utf8")
}

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().expect("vault id")
}
