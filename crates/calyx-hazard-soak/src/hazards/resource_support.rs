use calyx_aster::cf::{CfRouter, ColumnFamily};
use calyx_aster::compaction::{CompactionDebt, CompactionResult};
use calyx_aster::gc::{CompactionGcResult, TombstoneInventory, WalRecyclerResult};
use calyx_aster::mvcc::tombstone_value;
use calyx_aster::resource::{ResourceStatus, VramBudgetStatus};
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_aster::wal::{WalOptions, WalSegmentStatus};
use calyx_core::{Clock, Ts, VaultId};
use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

const BATCH: usize = 1_000;

#[derive(Clone, Debug)]
pub(super) struct SharedClock {
    now: Arc<AtomicU64>,
}

impl SharedClock {
    fn new(now: Ts) -> Self {
        Self {
            now: Arc::new(AtomicU64::new(now)),
        }
    }
}

impl Clock for SharedClock {
    fn now(&self) -> Ts {
        self.now.load(Ordering::Relaxed)
    }
}

pub(super) fn open_vault(
    vault_dir: &Path,
    now: Ts,
    salt: &[u8],
    memtable_byte_cap: usize,
    wal_options: Option<WalOptions>,
) -> Result<AsterVault<SharedClock>, String> {
    fs::create_dir_all(vault_dir).map_err(err)?;
    let mut options = VaultOptions {
        memtable_byte_cap,
        ..VaultOptions::default()
    };
    if let Some(wal_options) = wal_options {
        options.wal_options = wal_options;
    }
    AsterVault::new_durable_with_clock(
        vault_dir,
        vault_id(),
        salt.to_vec(),
        options,
        SharedClock::new(now),
    )
    .map_err(err)
}

pub(super) fn write_live_range(
    vault: &AsterVault<SharedClock>,
    prefix: &str,
    start: u64,
    end: u64,
    value_len: usize,
) -> Result<(), String> {
    let mut next = start;
    while next < end {
        let upper = (next + BATCH as u64).min(end);
        let rows =
            (next..upper).map(|id| (ColumnFamily::Base, key(prefix, id), value(id, value_len)));
        vault.write_cf_batch(rows).map_err(err)?;
        next = upper;
    }
    Ok(())
}

pub(super) fn write_tombstone_range(
    vault: &AsterVault<SharedClock>,
    prefix: &str,
    start: u64,
    end: u64,
) -> Result<(), String> {
    let mut next = start;
    while next < end {
        let upper = (next + BATCH as u64).min(end);
        let rows = (next..upper).map(|id| (ColumnFamily::Base, key(prefix, id), tombstone_value()));
        vault.write_cf_batch(rows).map_err(err)?;
        next = upper;
    }
    Ok(())
}

pub(super) fn count_readable(
    vault: &AsterVault<SharedClock>,
    prefix: &str,
    start: u64,
    end: u64,
) -> Result<usize, String> {
    let snapshot = vault.latest_seq();
    let mut found = 0usize;
    for id in start..end {
        if vault
            .read_cf_at(snapshot, ColumnFamily::Base, &key(prefix, id))
            .map_err(err)?
            .is_some()
        {
            found += 1;
        }
    }
    Ok(found)
}

pub(super) fn measure_read_p99_ns(
    vault: &AsterVault<SharedClock>,
    prefix: &str,
    start: u64,
    end: u64,
) -> Result<u128, String> {
    let snapshot = vault.latest_seq();
    let mut samples = Vec::new();
    for id in start..end {
        let started = Instant::now();
        let _ = vault
            .read_cf_at(snapshot, ColumnFamily::Base, &key(prefix, id))
            .map_err(err)?;
        samples.push(started.elapsed().as_nanos().max(1));
    }
    Ok(percentile(&mut samples, 99))
}

pub(super) fn percentile(samples: &mut [u128], pct: usize) -> u128 {
    if samples.is_empty() {
        return 0;
    }
    samples.sort_unstable();
    samples[(samples.len() * pct / 100).min(samples.len() - 1)]
}

pub(super) fn status(
    vault: &AsterVault<SharedClock>,
    vault_dir: &Path,
) -> Result<ResourceStatus, String> {
    vault.resource_status(vault_dir, vram()).map_err(err)
}

pub(super) fn max_memtable_used(router: &CfRouter) -> usize {
    router
        .memtable_usage_by_cf()
        .into_iter()
        .map(|(_, usage)| usage.used_bytes)
        .max()
        .unwrap_or(0)
}

pub(super) fn compaction_summary(result: &Option<CompactionResult>) -> (f64, u64, bool) {
    match result {
        Some(CompactionResult::Compacted(report)) => (
            report.write_amp_milli as f64 / 1_000.0,
            report.output_bytes,
            true,
        ),
        _ => (0.0, 0, false),
    }
}

pub(super) fn compaction_json(result: &Option<CompactionResult>) -> Value {
    match result {
        Some(CompactionResult::Compacted(report)) => json!({
            "kind": "compacted", "cf": report.cf.name(), "input_files": report.input_files,
            "input_bytes": report.input_bytes, "output_bytes": report.output_bytes,
            "logical_bytes": report.logical_bytes, "write_amp_milli": report.write_amp_milli,
            "debt_before": debt_json(&report.debt_before), "debt_after": debt_json(&report.debt_after)
        }),
        Some(CompactionResult::Skipped { debt }) => {
            json!({"kind": "skipped", "debt": debt_json(debt)})
        }
        None => json!({"kind": "no_durable_vault"}),
    }
}

pub(super) fn compaction_gc_json(result: &CompactionGcResult) -> Value {
    json!({
        "triggered": result.triggered, "rate_limited": result.rate_limited,
        "skipped_reason": result.skipped_reason, "error_code": result.error_code,
        "tombstone_ratio_before": result.tombstone_ratio_before,
        "tombstone_ratio_after": result.tombstone_ratio_after,
        "bytes_compacted": result.bytes_compacted, "bytes_freed": result.bytes_freed,
        "tombstones_removed": result.tombstones_removed, "write_amp_after": result.write_amp_after,
        "compaction_debt": result.compaction_debt, "compacted_cfs": result.compacted_cfs
    })
}

pub(super) fn inventory_json(inventory: &TombstoneInventory) -> Value {
    json!({
        "tombstone_keys": inventory.tombstone_keys(),
        "live_keys": inventory.live_keys(),
        "tombstone_ratio": inventory.tombstone_ratio(),
        "total_sst_bytes": inventory.total_sst_bytes(),
        "per_cf": inventory.per_cf.iter().map(|cf| json!({
            "cf": cf.cf_name, "sst_files": cf.sst_files, "sst_bytes": cf.sst_bytes,
            "live_keys": cf.live_keys, "tombstone_keys": cf.tombstone_keys,
            "tombstone_ratio": cf.tombstone_ratio()
        })).collect::<Vec<_>>()
    })
}

pub(super) fn wal_result_json(result: &WalRecyclerResult) -> Value {
    json!({
        "triggered": result.triggered, "rate_limited": result.rate_limited,
        "skipped_reason": result.skipped_reason, "error_code": result.error_code,
        "newest_durable_seq": result.newest_durable_seq,
        "wal_bytes_active_before": result.wal_bytes_active_before,
        "wal_bytes_active_after": result.wal_bytes_active_after,
        "recyclable_segments_before": result.recyclable_segments_before,
        "segments_recycled": result.segments_recycled, "bytes_recycled": result.bytes_recycled,
        "fsync_p99_us": result.fsync_p99_us
    })
}

pub(super) fn segment_json(segment: &WalSegmentStatus) -> Value {
    json!({
        "index": segment.index, "path": segment.path, "bytes": segment.bytes,
        "first_seq": segment.first_seq, "last_seq": segment.last_seq,
        "record_count": segment.record_count, "active": segment.active
    })
}

pub(super) fn key(prefix: &str, id: u64) -> Vec<u8> {
    format!("{prefix}-key-{id:06}").into_bytes()
}

pub(super) fn value(id: u64, len: usize) -> Vec<u8> {
    let mut value = format!("ph59-value-{id:06}:").into_bytes();
    value.resize(len, b'x');
    value
}

pub(super) fn case_dir(root: &Path, name: &str) -> Result<PathBuf, String> {
    let dir = root.join(name);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).map_err(err)?;
    Ok(dir)
}

pub(super) fn err(error: impl std::fmt::Display) -> String {
    error.to_string()
}

fn debt_json(debt: &CompactionDebt) -> Value {
    json!({
        "pending_bytes": debt.pending_bytes,
        "target_bytes": debt.target_bytes,
        "score_milli": debt.score_milli
    })
}

fn vram() -> VramBudgetStatus {
    VramBudgetStatus {
        budget_bytes: 0,
        used_bytes: 0,
        probe_warning: None,
    }
}

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().expect("vault id")
}
