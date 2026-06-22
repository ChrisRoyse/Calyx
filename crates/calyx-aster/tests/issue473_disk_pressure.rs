#![cfg(target_os = "linux")]

use calyx_aster::cf::ColumnFamily;
use calyx_aster::pressure::{
    CALYX_IO_ERROR, DiskPressureGuard, DiskSample, DiskSpaceProbe, SpillTrigger, TempFile,
};
use calyx_aster::resource::VramBudgetStatus;
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{Clock, FixedClock, Result, VaultId};
use serde_json::json;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

mod fsv_support;
use fsv_support::{env_or_prepared_temp_root, prepared_temp_root};

#[derive(Debug)]
struct SharedProbe {
    sample: Mutex<Result<DiskSample>>,
}

impl SharedProbe {
    fn new(sample: DiskSample) -> Self {
        Self {
            sample: Mutex::new(Ok(sample)),
        }
    }

    fn set(&self, sample: DiskSample) {
        *self.sample.lock().unwrap() = Ok(sample);
    }
}

impl DiskSpaceProbe for SharedProbe {
    fn sample(&self, _path: &Path) -> Result<DiskSample> {
        self.sample.lock().unwrap().clone()
    }
}

#[test]
fn write_path_rejects_before_wal_and_allows_after_pressure_drops() {
    let dir = test_dir("write-path");
    let clock = clock();
    let probe = Arc::new(SharedProbe::new(DiskSample {
        blocks: 100,
        blocks_available: 10,
    }));
    let (sender, receiver) = mpsc::channel();
    let guard = DiskPressureGuard::with_probe(&dir, 0.85, clock.clone(), probe.clone())
        .with_spill_trigger(SpillTrigger::new(&dir, sender, clock));
    let vault = open_vault(
        &dir,
        VaultOptions {
            disk_pressure_guard: Some(guard),
            ..VaultOptions::default()
        },
    );
    let before_wal = wal_bytes(&dir);

    let error = vault
        .write_cf(ColumnFamily::Base, b"blocked".to_vec(), b"value".to_vec())
        .unwrap_err();

    assert_eq!(error.code, "CALYX_DISK_PRESSURE");
    assert_eq!(wal_bytes(&dir), before_wal);
    assert_eq!(vault.latest_seq(), 0);
    assert!(
        vault
            .read_cf_at(vault.latest_seq(), ColumnFamily::Base, b"blocked")
            .unwrap()
            .is_none()
    );
    assert!(receiver.recv_timeout(Duration::from_secs(1)).is_ok());
    let metrics = vault
        .resource_status(&dir, vram())
        .unwrap()
        .to_metrics_text("issue473");
    assert!(metrics.contains("calyx_disk_pressure_events_total{vault=\"issue473\"} 1"));

    probe.set(DiskSample {
        blocks: 100,
        blocks_available: 30,
    });
    let seq = vault
        .write_cf(ColumnFamily::Base, b"allowed".to_vec(), b"value".to_vec())
        .unwrap();
    assert_eq!(seq, 1);
    assert_eq!(
        vault
            .read_cf_at(seq, ColumnFamily::Base, b"allowed")
            .unwrap(),
        Some(b"value".to_vec())
    );
    fs::remove_dir_all(dir).unwrap();
}

#[test]
#[ignore = "manual FSV driver; run, then read evidence files separately"]
fn manual_real_statvfs_disk_pressure_readback() {
    let root = env_or_prepared_temp_root("CALYX_FSV_ROOT", "calyx-issue473", "fsv-readback");
    let vault_dir = root.join("vault");
    fs::create_dir_all(&vault_dir).unwrap();
    fs::write(
        root.join("cleanup-tag.txt"),
        b"issue473 synthetic FSV data\n",
    )
    .unwrap();
    let clock = clock();

    let ok_guard = DiskPressureGuard::new(&vault_dir, 1.0, clock.clone());
    let ok_vault = open_vault(
        &vault_dir,
        VaultOptions {
            disk_pressure_guard: Some(ok_guard),
            ..VaultOptions::default()
        },
    );
    fs::write(
        root.join("metrics-before.prom"),
        ok_vault
            .resource_status(&vault_dir, vram())
            .unwrap()
            .to_metrics_text("issue473"),
    )
    .unwrap();
    let ok_seq = ok_vault
        .write_cf(ColumnFamily::Base, b"ok-key".to_vec(), b"ok-value".to_vec())
        .unwrap();
    assert_eq!(
        ok_vault
            .read_cf_at(ok_seq, ColumnFamily::Base, b"ok-key")
            .unwrap(),
        Some(b"ok-value".to_vec())
    );
    let wal_before_reject = wal_bytes(&vault_dir);
    fs::write(
        root.join("wal-before-reject.txt"),
        wal_before_reject.to_string(),
    )
    .unwrap();
    drop(ok_vault);

    let (sender, receiver) = mpsc::channel();
    let pressure_guard = DiskPressureGuard::new(&vault_dir, 0.0, clock.clone())
        .with_spill_trigger(SpillTrigger::new(&vault_dir, sender, clock.clone()));
    let pressure_vault = open_vault(
        &vault_dir,
        VaultOptions {
            disk_pressure_guard: Some(pressure_guard),
            ..VaultOptions::default()
        },
    );
    let before_reject_seq = pressure_vault.latest_seq();
    let pressure_error = pressure_vault
        .write_cf(
            ColumnFamily::Base,
            b"blocked-key".to_vec(),
            b"blocked-value".to_vec(),
        )
        .unwrap_err();
    let wal_after_reject = wal_bytes(&vault_dir);
    let blocked_absent = pressure_vault
        .read_cf_at(
            pressure_vault.latest_seq(),
            ColumnFamily::Base,
            b"blocked-key",
        )
        .unwrap()
        .is_none();
    let spill_request = receiver.recv_timeout(Duration::from_secs(1)).unwrap();
    let metrics_after = pressure_vault
        .resource_status(&vault_dir, vram())
        .unwrap()
        .to_metrics_text("issue473");
    fs::write(root.join("metrics-after-pressure.prom"), &metrics_after).unwrap();
    drop(pressure_vault);

    let invalid_error = DiskPressureGuard::new(root.join("missing-path"), 0.85, clock.clone())
        .check()
        .unwrap_err();
    let temp = TempFile::in_dataset(&vault_dir).unwrap();
    let temp_parent_matches = temp.path().parent() == Some(vault_dir.as_path());
    let temp_path = temp.path().to_path_buf();
    drop(temp);

    let clear_guard = DiskPressureGuard::new(&vault_dir, 1.0, clock);
    let clear_vault = open_vault(
        &vault_dir,
        VaultOptions {
            disk_pressure_guard: Some(clear_guard),
            ..VaultOptions::default()
        },
    );
    let clear_seq = clear_vault
        .write_cf(
            ColumnFamily::Base,
            b"clear-key".to_vec(),
            b"clear-value".to_vec(),
        )
        .unwrap();
    let clear_read = clear_vault
        .read_cf_at(clear_seq, ColumnFamily::Base, b"clear-key")
        .unwrap();
    let clear_read_hex = clear_read.as_deref().map(hex);
    let summary = json!({
        "vault_dir": vault_dir,
        "ok_seq": ok_seq,
        "before_reject_seq": before_reject_seq,
        "pressure_error_code": pressure_error.code,
        "pressure_message": pressure_error.message,
        "wal_before_reject": wal_before_reject,
        "wal_after_reject": wal_after_reject,
        "wal_unchanged_after_reject": wal_before_reject == wal_after_reject,
        "blocked_absent": blocked_absent,
        "spill_request_hotpool": spill_request.hotpool_path,
        "invalid_path_error_code": invalid_error.code,
        "temp_parent_matches_dataset": temp_parent_matches,
        "temp_removed_on_drop": !temp_path.exists(),
        "clear_seq": clear_seq,
        "clear_read_hex": clear_read_hex,
        "disk_pressure_metric_present": metrics_after
            .contains("calyx_disk_pressure_events_total{vault=\"issue473\"} 1"),
    });
    fs::write(
        root.join("issue473-disk-pressure-readback.json"),
        serde_json::to_vec_pretty(&summary).unwrap(),
    )
    .unwrap();
    assert_eq!(pressure_error.code, "CALYX_DISK_PRESSURE");
    assert_eq!(invalid_error.code, CALYX_IO_ERROR);
    assert_eq!(wal_before_reject, wal_after_reject);
    assert!(blocked_absent);
    assert!(temp_parent_matches);
    assert_eq!(clear_read, Some(b"clear-value".to_vec()));
}

fn test_dir(name: &str) -> PathBuf {
    prepared_temp_root("calyx-issue473", name)
}

fn open_vault(dir: &Path, options: VaultOptions) -> AsterVault<FixedClock> {
    AsterVault::new_durable_with_clock(dir, vault_id(), b"issue473".to_vec(), options, *fixed())
        .unwrap()
}

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap()
}

fn clock() -> Arc<dyn Clock> {
    Arc::new(*fixed())
}

fn fixed() -> &'static FixedClock {
    static CLOCK: FixedClock = FixedClock::new(1_785_000_473);
    &CLOCK
}

fn vram() -> VramBudgetStatus {
    VramBudgetStatus {
        budget_bytes: 0,
        used_bytes: 0,
        probe_warning: None,
    }
}

fn wal_bytes(vault_dir: &Path) -> u64 {
    let wal_dir = vault_dir.join("wal");
    if !wal_dir.is_dir() {
        return 0;
    }
    fs::read_dir(wal_dir)
        .unwrap()
        .filter_map(|entry| {
            let path = entry.unwrap().path();
            (path.extension().and_then(|value| value.to_str()) == Some("wal")).then_some(path)
        })
        .map(|path| fs::metadata(path).unwrap().len())
        .sum()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
