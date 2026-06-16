use calyx_aster::cf::ColumnFamily;
use calyx_aster::pressure::{
    DEFAULT_HIGH_WATER_RATIO, DiskPressureGuard, DiskSample, DiskSpaceProbe, DiskStatus,
    SpillRequest, SpillTrigger,
};
use calyx_aster::resource::VramBudgetStatus;
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_aster::wal::WalOptions;
use calyx_core::Result as CalyxResult;
use serde_json::json;
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::time::Duration;

use super::operational_support::{MutableClock, quota_vault_id, read_rate};
use super::resource::ProbeResult;
use super::resource_support::{
    case_dir, count_readable, err, key, measure_read_p99_ns, open_vault, value, write_live_range,
};

const START_TS: u64 = 1_800_500_000_000;
const MEMTABLE_BYTES: usize = 64 * 1024 * 1024;

pub(super) fn probe_h17_disk_pressure(root: &Path) -> ProbeResult {
    let dir = case_dir(root, "h17_disk_pressure")?;
    let vault_dir = dir.join("vault");
    let clock = MutableClock::new(START_TS + 17);
    let probe = Arc::new(AtomicDiskProbe::new(100, 20));
    let (spill_tx, spill_rx) = mpsc::channel();
    let guard_probe: Arc<dyn DiskSpaceProbe> = probe.clone();
    let guard = DiskPressureGuard::with_probe(
        &vault_dir,
        DEFAULT_HIGH_WATER_RATIO,
        Arc::new(clock.clone()),
        guard_probe,
    )
    .with_spill_trigger(SpillTrigger::new(
        &vault_dir,
        spill_tx,
        Arc::new(clock.clone()),
    ));
    let mut options = VaultOptions {
        memtable_byte_cap: MEMTABLE_BYTES,
        disk_pressure_guard: Some(guard),
        ..VaultOptions::default()
    };
    options.wal_options = WalOptions {
        max_segment_bytes: 16 * 1024,
        group_commit_window: Duration::ZERO,
    };
    let vault = AsterVault::new_durable_with_clock(
        &vault_dir,
        quota_vault_id(17),
        b"ph59-h17".to_vec(),
        options,
        clock,
    )
    .map_err(err)?;

    vault
        .write_cf(ColumnFamily::Base, key("h17-ok", 0), value(0, 64))
        .map_err(err)?;
    let before_pressure_seq = vault.latest_seq();
    probe.set_available(10);
    let rejected = vault
        .write_cf(ColumnFamily::Base, key("h17-rejected", 0), value(1, 64))
        .expect_err("disk pressure must fail closed");
    let spill = spill_rx.try_recv().ok();
    let after_reject_seq = vault.latest_seq();
    let rejected_absent = vault
        .read_cf_at(
            after_reject_seq,
            ColumnFamily::Base,
            &key("h17-rejected", 0),
        )
        .map_err(err)?
        .is_none();

    probe.set_available(16);
    vault
        .write_cf(ColumnFamily::Base, key("h17-recovered", 0), value(2, 64))
        .map_err(err)?;
    vault.flush().map_err(err)?;
    let resource = vault
        .resource_status(&vault_dir, empty_vram())
        .map_err(err)?;
    let recovered_present = vault
        .read_cf_at(
            vault.latest_seq(),
            ColumnFamily::Base,
            &key("h17-recovered", 0),
        )
        .map_err(err)?
        .is_some();
    let boundary = boundary_readback(&vault_dir, &clock_for_boundary())?;
    let passed = before_pressure_seq == after_reject_seq
        && rejected.code == "CALYX_DISK_PRESSURE"
        && rejected_absent
        && recovered_present
        && spill.is_some()
        && resource.backpressure.disk_pressure_events_total == 1
        && boundary.boundary_rejected;

    Ok((
        passed,
        json!({
            "trigger": "durable Aster write admitted at 80% used, rejected at 90% used, recovered at 84% used",
            "expected": {
                "pressure_error_code": "CALYX_DISK_PRESSURE",
                "seq_not_advanced_on_reject": true,
                "rejected_key_absent": true,
                "spill_requested": true
            },
            "actual": {
                "seq_before_pressure": before_pressure_seq,
                "seq_after_reject": after_reject_seq,
                "rejected_error_code": rejected.code,
                "rejected_key_absent": rejected_absent,
                "recovered_key_present": recovered_present,
                "spill_request": spill_json(spill),
                "resource_status": resource,
                "boundary": boundary,
                "panic_free": true
            },
            "metrics_text": resource.to_metrics_text("ph59-h17")
        }),
    ))
}

pub(super) fn probe_h18_arc_read_thrash(root: &Path) -> ProbeResult {
    let dir = case_dir(root, "h18_arc_read_thrash")?;
    let vault_dir = dir.join("vault");
    let vault = open_vault(&vault_dir, START_TS + 18, b"ph59-h18", MEMTABLE_BYTES, None)?;
    write_live_range(&vault, "h18", 0, 4_096, 512)?;
    vault.flush().map_err(err)?;
    let readback_before = count_readable(&vault, "h18", 0, 4_096)?;
    let baseline = read_rate(&vault, "h18", 0, 512, Duration::from_millis(120))?;
    let p99_before = measure_read_p99_ns(&vault, "h18", 0, 512)?;
    let thrash_path = dir.join("arc_pressure.bin");
    let thrash_bytes = write_thrash_file(&thrash_path, 8 * 1024 * 1024)?;
    let checksum = churn_file(&thrash_path, 3)?;
    let churned = read_rate(&vault, "h18", 512, 2_048, Duration::from_millis(150))?;
    let recovered = read_rate(&vault, "h18", 0, 512, Duration::from_millis(120))?;
    let p99_after = measure_read_p99_ns(&vault, "h18", 0, 512)?;
    let readback_after = count_readable(&vault, "h18", 0, 4_096)?;
    let ratio = recovered.ops_per_sec / baseline.ops_per_sec.max(1.0);
    let passed = readback_before == 4_096
        && readback_after == 4_096
        && baseline.hits == baseline.reads
        && churned.hits == churned.reads
        && recovered.hits == recovered.reads
        && ratio >= 0.05
        && p99_after <= p99_before.saturating_mul(100).max(1)
        && checksum != 0;

    Ok((
        passed,
        json!({
            "trigger": "Aster base CF read loop before/after deterministic filesystem churn file",
            "expected": {"all_db_reads_hit": true, "recovery_ops_ratio_gte": 0.05, "read_p99_not_unbounded": true},
            "actual": {
                "readback_before": readback_before,
                "readback_after": readback_after,
                "baseline": baseline,
                "during_churn": churned,
                "recovered": recovered,
                "recovery_ops_ratio": ratio,
                "read_p99_before_ns": p99_before,
                "read_p99_after_ns": p99_after,
                "thrash_file_bytes": thrash_bytes,
                "thrash_checksum": checksum,
                "panic_free": true
            },
            "metrics_text": format!(
                "calyx_arc_thrash_bytes_total{{vault=\"ph59-h18\"}} {thrash_bytes}\ncalyx_read_throughput_ratio{{vault=\"ph59-h18\"}} {ratio:.6}\n"
            )
        }),
    ))
}

pub(super) fn probe_h19_clock_skew(root: &Path) -> ProbeResult {
    let dir = case_dir(root, "h19_clock_skew")?;
    let vault_dir = dir.join("vault");
    let clock = MutableClock::new(START_TS + 19_000);
    let vault = AsterVault::new_durable_with_clock(
        &vault_dir,
        quota_vault_id(19),
        b"ph59-h19".to_vec(),
        VaultOptions {
            memtable_byte_cap: MEMTABLE_BYTES,
            ..VaultOptions::default()
        },
        clock.clone(),
    )
    .map_err(err)?;
    for id in 0..20 {
        vault
            .write_cf(ColumnFamily::Base, key("h19-forward", id), value(id, 48))
            .map_err(err)?;
    }
    let forward_last_seq = vault
        .seq_for_key(ColumnFamily::Base, &key("h19-forward", 19))
        .map_err(err)?;
    clock.set(START_TS - 10_000);
    for id in 0..20 {
        vault
            .write_cf(ColumnFamily::Base, key("h19-backward", id), value(id, 48))
            .map_err(err)?;
    }
    clock.set(0);
    vault
        .write_cf(ColumnFamily::Base, key("h19-zero-clock", 0), value(40, 48))
        .map_err(err)?;
    vault.flush().map_err(err)?;
    let latest_before_reopen = vault.latest_seq();
    let seqs = collect_h19_seqs(&vault)?;
    drop(vault);
    let reopened = AsterVault::new_durable_with_clock(
        &vault_dir,
        quota_vault_id(19),
        b"ph59-h19".to_vec(),
        VaultOptions {
            memtable_byte_cap: MEMTABLE_BYTES,
            ..VaultOptions::default()
        },
        clock,
    )
    .map_err(err)?;
    let rows_after_reopen = count_h19_rows(&reopened)?;
    let time_index_rows = reopened
        .scan_cf_at(reopened.latest_seq(), ColumnFamily::TimeIndex)
        .map_err(err)?
        .len();
    let strictly_increasing = seqs.windows(2).all(|pair| pair[0] < pair[1]);
    let zero_clock_seq = reopened
        .seq_for_key(ColumnFamily::Base, &key("h19-zero-clock", 0))
        .map_err(err)?
        .unwrap_or(0);
    let passed = forward_last_seq.is_some()
        && strictly_increasing
        && rows_after_reopen == 41
        && reopened.latest_seq() == latest_before_reopen
        && zero_clock_seq == latest_before_reopen
        && time_index_rows >= 41;

    Ok((
        passed,
        json!({
            "trigger": "durable writes with forward clock, backwards clock, then zero clock",
            "expected": {"mvcc_seq_strictly_increases": true, "all_rows_reopen": 41, "latest_seq_survives_reopen": true},
            "actual": {
                "forward_last_seq": forward_last_seq,
                "seqs": seqs,
                "strictly_increasing": strictly_increasing,
                "latest_before_reopen": latest_before_reopen,
                "latest_after_reopen": reopened.latest_seq(),
                "rows_after_reopen": rows_after_reopen,
                "zero_clock_seq": zero_clock_seq,
                "time_index_rows": time_index_rows,
                "panic_free": true
            },
            "metrics_text": format!(
                "calyx_clock_skew_rows_verified{{vault=\"ph59-h19\"}} {rows_after_reopen}\ncalyx_clock_skew_latest_seq{{vault=\"ph59-h19\"}} {}\n",
                reopened.latest_seq()
            )
        }),
    ))
}

#[derive(Debug)]
struct AtomicDiskProbe {
    blocks: u64,
    available: AtomicU64,
}

impl AtomicDiskProbe {
    fn new(blocks: u64, available: u64) -> Self {
        Self {
            blocks,
            available: AtomicU64::new(available),
        }
    }

    fn set_available(&self, available: u64) {
        self.available.store(available, Ordering::SeqCst);
    }
}

impl DiskSpaceProbe for AtomicDiskProbe {
    fn sample(&self, _path: &Path) -> CalyxResult<DiskSample> {
        Ok(DiskSample {
            blocks: self.blocks,
            blocks_available: self.available.load(Ordering::SeqCst),
        })
    }
}

#[derive(serde::Serialize)]
struct BoundaryReadback {
    boundary_rejected: bool,
    boundary_error_code: Option<String>,
    below_boundary_status: String,
}

fn boundary_readback(
    path: &Path,
    clock: &Arc<dyn calyx_core::Clock>,
) -> Result<BoundaryReadback, String> {
    let probe = Arc::new(AtomicDiskProbe::new(100, 15));
    let guard_probe: Arc<dyn DiskSpaceProbe> = probe.clone();
    let guard = DiskPressureGuard::with_probe(
        path,
        DEFAULT_HIGH_WATER_RATIO,
        Arc::clone(clock),
        guard_probe,
    );
    let boundary_error = guard.check().unwrap_err();
    probe.set_available(16);
    let below_boundary_status = match guard.check().map_err(err)? {
        DiskStatus::Ok { used_ratio, .. } => format!("ok:{used_ratio:.2}"),
    };
    Ok(BoundaryReadback {
        boundary_rejected: boundary_error.code == "CALYX_DISK_PRESSURE",
        boundary_error_code: Some(boundary_error.code.to_string()),
        below_boundary_status,
    })
}

fn clock_for_boundary() -> Arc<dyn calyx_core::Clock> {
    Arc::new(MutableClock::new(START_TS + 17))
}

fn empty_vram() -> VramBudgetStatus {
    VramBudgetStatus {
        budget_bytes: 0,
        used_bytes: 0,
        probe_warning: None,
    }
}

fn spill_json(spill: Option<SpillRequest>) -> serde_json::Value {
    match spill {
        Some(request) => {
            json!({"hotpool_path": request.hotpool_path, "requested_at": request.requested_at})
        }
        None => json!(null),
    }
}

fn write_thrash_file(path: &Path, bytes: usize) -> Result<usize, String> {
    let mut file = File::create(path).map_err(err)?;
    let mut chunk = vec![0_u8; 64 * 1024];
    let mut written = 0usize;
    while written < bytes {
        for (idx, byte) in chunk.iter_mut().enumerate() {
            *byte = ((written + idx) as u8).wrapping_mul(31).wrapping_add(17);
        }
        let len = chunk.len().min(bytes - written);
        file.write_all(&chunk[..len]).map_err(err)?;
        written += len;
    }
    file.sync_all().map_err(err)?;
    Ok(written)
}

fn churn_file(path: &Path, cycles: usize) -> Result<u64, String> {
    let mut file = File::open(path).map_err(err)?;
    let len = fs::metadata(path).map_err(err)?.len();
    let blocks = (len / 4096).max(1);
    let mut buf = [0_u8; 4096];
    let mut checksum = len ^ 0x9E37_79B9_7F4A_7C15;
    for cycle in 0..cycles as u64 {
        for idx in 0..blocks {
            let block = (idx.wrapping_mul(4_099).wrapping_add(cycle * 97)) % blocks;
            file.seek(SeekFrom::Start(block * 4096)).map_err(err)?;
            let read = file.read(&mut buf).map_err(err)?;
            for byte in &buf[..read] {
                checksum = checksum
                    .wrapping_mul(1_099_511_628_211)
                    .wrapping_add(u64::from(*byte) + 1)
                    .rotate_left(5);
            }
        }
    }
    Ok(checksum)
}

fn collect_h19_seqs<C>(vault: &AsterVault<C>) -> Result<Vec<u64>, String>
where
    C: calyx_core::Clock,
{
    let mut seqs = Vec::new();
    for id in 0..20 {
        seqs.push(
            vault
                .seq_for_key(ColumnFamily::Base, &key("h19-forward", id))
                .map_err(err)?
                .unwrap_or(0),
        );
    }
    for id in 0..20 {
        seqs.push(
            vault
                .seq_for_key(ColumnFamily::Base, &key("h19-backward", id))
                .map_err(err)?
                .unwrap_or(0),
        );
    }
    seqs.push(
        vault
            .seq_for_key(ColumnFamily::Base, &key("h19-zero-clock", 0))
            .map_err(err)?
            .unwrap_or(0),
    );
    Ok(seqs)
}

fn count_h19_rows<C>(vault: &AsterVault<C>) -> Result<usize, String>
where
    C: calyx_core::Clock,
{
    let forward = (0..20)
        .map(|id| {
            vault
                .read_cf_at(
                    vault.latest_seq(),
                    ColumnFamily::Base,
                    &key("h19-forward", id),
                )
                .map_err(err)
        })
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .filter(Option::is_some)
        .count();
    let backward = (0..20)
        .map(|id| {
            vault
                .read_cf_at(
                    vault.latest_seq(),
                    ColumnFamily::Base,
                    &key("h19-backward", id),
                )
                .map_err(err)
        })
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .filter(Option::is_some)
        .count();
    let zero = vault
        .read_cf_at(
            vault.latest_seq(),
            ColumnFamily::Base,
            &key("h19-zero-clock", 0),
        )
        .map_err(err)?
        .is_some();
    Ok(forward + backward + usize::from(zero))
}
