use calyx_aster::cf::{CfRouter, ColumnFamily};
use calyx_aster::gc::{
    CompactionGcReclaimer, VaultCompactionGcTarget, WalRecycler, scan_tombstone_inventory,
};
use calyx_aster::resource::heap_rss_bytes;
use calyx_aster::wal::{Wal, WalOptions};
use serde::Serialize;
use serde_json::{Value, json};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;
use std::time::{Duration, Instant};

use super::resource_support::*;

const START_TS: u64 = 1_800_100_000_000;
const FSV_MEMTABLE_BYTES: usize = 64 * 1024 * 1024;
pub(super) type ProbeResult = Result<(bool, Value), String>;

#[derive(Clone, Debug, Serialize)]
pub struct HazardResult {
    pub hazard_id: u8,
    pub name: &'static str,
    pub passed: bool,
    pub evidence: Value,
}

pub fn run_hazards_1_5(root: &Path) -> Vec<HazardResult> {
    [
        (
            1,
            "write amplification / compaction storm",
            probe_h1 as fn(&Path) -> ProbeResult,
        ),
        (2, "memtable flush stall", probe_h2),
        (3, "tombstone buildup", probe_h3),
        (4, "fsync latency spike", probe_h4),
        (5, "WAL bloat", probe_h5),
    ]
    .into_iter()
    .map(|(id, name, probe)| run_probe(root, id, name, probe))
    .collect()
}

pub(super) fn run_probe(
    root: &Path,
    hazard_id: u8,
    name: &'static str,
    probe: fn(&Path) -> ProbeResult,
) -> HazardResult {
    match catch_unwind(AssertUnwindSafe(|| probe(root))) {
        Ok(Ok((passed, evidence))) => HazardResult {
            hazard_id,
            name,
            passed,
            evidence,
        },
        Ok(Err(error)) => HazardResult {
            hazard_id,
            name,
            passed: false,
            evidence: json!({"error": error, "panic_free": true}),
        },
        Err(payload) => HazardResult {
            hazard_id,
            name,
            passed: false,
            evidence: json!({"panic": panic_text(payload), "panic_free": false}),
        },
    }
}

fn probe_h1(root: &Path) -> ProbeResult {
    let dir = case_dir(root, "h1_compaction_storm")?;
    let vault_dir = dir.join("vault");
    let vault = open_vault(
        &vault_dir,
        START_TS + 100,
        b"ph59-h1",
        FSV_MEMTABLE_BYTES,
        None,
    )?;
    write_live_range(&vault, "h1", 0, 10_000, 256)?;
    vault.flush().map_err(err)?;
    let before = status(&vault, &vault_dir)?;
    let baseline_p99 = measure_read_p99_ns(&vault, "h1", 0, 1_000)?;
    write_live_range(&vault, "h1", 10_000, 30_000, 256)?;
    vault.flush().map_err(err)?;
    let storm = status(&vault, &vault_dir)?;
    let started = Instant::now();
    let compaction = vault.compact_cf_once(ColumnFamily::Base).map_err(err)?;
    let elapsed_ms = started.elapsed().as_millis();
    let after_p99 = measure_read_p99_ns(&vault, "h1", 20_000, 21_000)?;
    let after = status(&vault, &vault_dir)?;
    let (write_amp, output_bytes, compacted) = compaction_summary(&compaction);
    let p99_ok = after_p99 <= baseline_p99.saturating_mul(2).max(1);
    let debt_ok = after.compaction.total_pending_bytes
        <= storm
            .compaction
            .total_pending_bytes
            .saturating_add(output_bytes)
            .saturating_add(8 * 1024);
    let passed = compacted && write_amp <= 10.0 && p99_ok && debt_ok;
    Ok((
        passed,
        json!({
            "trigger": "30k write-heavy durable rows followed by base CF compaction",
            "expected": {"write_amp_lte": 10.0, "serving_p99_lte_2x": true, "debt_not_unbounded": true},
            "actual": {
                "status_before": before, "status_storm": storm, "status_after": after,
                "compaction": compaction_json(&compaction), "compaction_elapsed_ms": elapsed_ms,
                "write_amp": write_amp, "serving_p99_baseline_ns": baseline_p99,
                "serving_p99_after_ns": after_p99, "debt_after_lte_storm_plus_output": debt_ok,
                "panic_free": true
            },
            "metrics_text": format!("calyx_write_amp{{vault=\"ph59-h1\"}} {write_amp:.6}\ncalyx_compaction_debt{{vault=\"ph59-h1\"}} {}\n", after.compaction.total_pending_bytes)
        }),
    ))
}

fn probe_h2(root: &Path) -> ProbeResult {
    let dir = case_dir(root, "h2_flush_stall")?;
    let mut router = CfRouter::open(dir.join("vault"), 4 * 1024).map_err(err)?;
    let rss_before = heap_rss_bytes().map_err(err)?;
    let mut max_used = 0usize;
    let mut buckets = Vec::new();
    for bucket in 0..5 {
        let mut acks = 0usize;
        for idx in 0..200 {
            let key = format!("h2-{bucket:02}-{idx:03}");
            router
                .put(ColumnFamily::Base, key.as_bytes(), &[0xAB; 512])
                .map_err(err)?;
            acks += 1;
            max_used = max_used.max(max_memtable_used(&router));
        }
        buckets.push(acks);
    }
    let rejected = router
        .put(ColumnFamily::Base, b"h2-too-large", &[0xCD; 8 * 1024])
        .expect_err("oversized row must fail closed");
    let rss_after = heap_rss_bytes().map_err(err)?;
    let counters = router.resource_counters().snapshot();
    let usage = router.memtable_usage_by_cf();
    let passed = buckets.iter().all(|acks| *acks > 0)
        && max_used <= 4 * 1024
        && rejected.code == "CALYX_BACKPRESSURE"
        && counters.memtable_absorbed_total > 0
        && counters.memtable_rejected_total > 0
        && rss_after <= rss_before.saturating_add(32 * 1024 * 1024);
    Ok((
        passed,
        json!({
            "trigger": "bounded-CF-router write flood plus oversized row",
            "expected": {"ack_rate_min_gt_zero": true, "memtable_used_lte_cap": true, "fail_closed_code": "CALYX_BACKPRESSURE"},
            "actual": {
                "ack_buckets": buckets, "ack_rate_min": buckets.iter().min().copied().unwrap_or(0),
                "max_memtable_used_bytes": max_used, "memtable_cap_bytes": 4 * 1024,
                "backpressure": counters, "rejected_error_code": rejected.code,
                "rss_before": rss_before, "rss_after": rss_after,
                "usage": usage.iter().map(|(cf, useg)| json!({"cf": cf.name(), "used": useg.used_bytes, "cap": useg.cap_bytes})).collect::<Vec<_>>(),
                "panic_free": true
            },
            "metrics_text": format!("calyx_memtable_used_bytes{{vault=\"ph59-h2\",cf=\"base\"}} {max_used}\ncalyx_backpressure_events_total{{vault=\"ph59-h2\",source=\"memtable_rejected\"}} {}\n", counters.memtable_rejected_total)
        }),
    ))
}

fn probe_h3(root: &Path) -> ProbeResult {
    let dir = case_dir(root, "h3_tombstone_buildup")?;
    let vault_dir = dir.join("vault");
    let vault = open_vault(
        &vault_dir,
        START_TS + 300,
        b"ph59-h3",
        FSV_MEMTABLE_BYTES,
        None,
    )?;
    write_live_range(&vault, "h3", 0, 100_000, 24)?;
    vault.flush().map_err(err)?;
    write_tombstone_range(&vault, "h3", 0, 70_000)?;
    vault.flush().map_err(err)?;
    let before = scan_tombstone_inventory(&vault_dir).map_err(err)?;
    let target = VaultCompactionGcTarget {
        vault: &vault,
        vault_dir: &vault_dir,
    };
    let mut reclaimer = CompactionGcReclaimer::with_limits(0.5, 1, 1_000_000_000, 0);
    reclaimer.tombstone_ratio_trigger = 0.4;
    let mut results = Vec::new();
    let mut ratios = vec![before.tombstone_ratio()];
    for pass in 0..5 {
        let result = reclaimer.maybe_trigger_at(&target, 0.8, pass * 1_000);
        ratios.push(result.tombstone_ratio_after);
        results.push(compaction_gc_json(&result));
    }
    let after = scan_tombstone_inventory(&vault_dir).map_err(err)?;
    let passed = before.tombstone_ratio() > reclaimer.tombstone_ratio_trigger
        && after.tombstone_ratio() <= 0.1;
    Ok((
        passed,
        json!({
            "trigger": "100k base rows, 70k MVCC tombstones, five compaction GC sweeps",
            "expected": {
                "ratio_before_gt_trigger": reclaimer.tombstone_ratio_trigger,
                "ratio_after_lte_0_1": true
            },
            "actual": {
                "ratio_series": ratios, "before": inventory_json(&before),
                "after": inventory_json(&after), "results": results,
                "panic_free": true
            },
            "metrics_text": format!("calyx_tombstone_ratio{{vault=\"ph59-h3\"}} {:.6}\n", after.tombstone_ratio())
        }),
    ))
}

fn probe_h4(root: &Path) -> ProbeResult {
    let dir = case_dir(root, "h4_fsync_spike")?;
    let vault_dir = dir.join("vault");
    let wal_options = WalOptions {
        max_segment_bytes: 512,
        group_commit_window: Duration::ZERO,
    };
    let vault = open_vault(
        &vault_dir,
        START_TS + 400,
        b"ph59-h4",
        FSV_MEMTABLE_BYTES,
        Some(wal_options),
    )?;
    let mut write_ns = Vec::new();
    for id in 0..100 {
        let started = Instant::now();
        vault
            .write_cf(ColumnFamily::Base, key("h4", id), value(id, 96))
            .map_err(err)?;
        write_ns.push(started.elapsed().as_nanos().max(1));
    }
    vault.flush().map_err(err)?;
    let seq = vault.latest_seq();
    let readable_before_drop = count_readable(&vault, "h4", 0, 100)?;
    drop(vault);
    let mut wal = Wal::open(vault_dir.join("wal"), wal_options).map_err(err)?;
    let before_bytes = wal.total_segment_bytes().map_err(err)?;
    let recycler = WalRecycler::with_limits(64, 64, Duration::from_millis(1_000));
    recycler.set_fsync_p99_us(15_000);
    let guarded = recycler.run_once_at(&mut wal, seq, 1_000);
    recycler.set_fsync_p99_us(0);
    let backed_off = recycler.run_once_at(&mut wal, seq, 2_999);
    let recovered = recycler.run_once_at(&mut wal, seq, 3_000);
    drop(wal);
    let reopened = open_vault(
        &vault_dir,
        START_TS + 401,
        b"ph59-h4",
        FSV_MEMTABLE_BYTES,
        Some(wal_options),
    )?;
    let readable_after_reopen = count_readable(&reopened, "h4", 0, 100)?;
    let passed = readable_before_drop == 100
        && readable_after_reopen == 100
        && guarded.skipped_reason == Some("fsync_p99_guard")
        && backed_off.skipped_reason == Some("fsync_backoff_active")
        && recovered.triggered
        && recovered.fsync_p99_us < 10_000;
    Ok((
        passed,
        json!({
            "trigger": "100 durable writes plus injected fsync_p99 guard spike",
            "expected": {"acked_writes_readable": 100, "fsync_p99_recovers_lt_us": 10000, "no_data_loss": true},
            "actual": {
                "acked_writes": 100, "readable_before_drop": readable_before_drop,
                "readable_after_reopen": readable_after_reopen, "write_p99_ns": percentile(&mut write_ns, 99),
                "wal_bytes_before_recycle": before_bytes, "guarded": wal_result_json(&guarded),
                "backed_off": wal_result_json(&backed_off), "recovered": wal_result_json(&recovered),
                "panic_free": true
            },
            "metrics_text": guarded.to_metrics_text("ph59-h4") + &recovered.to_metrics_text("ph59-h4")
        }),
    ))
}

fn probe_h5(root: &Path) -> ProbeResult {
    let dir = case_dir(root, "h5_wal_bloat")?;
    let vault_dir = dir.join("vault");
    let max_segment = 16 * 1024;
    let wal_options = WalOptions {
        max_segment_bytes: max_segment,
        group_commit_window: Duration::ZERO,
    };
    let vault = open_vault(
        &vault_dir,
        START_TS + 500,
        b"ph59-h5",
        FSV_MEMTABLE_BYTES,
        Some(wal_options),
    )?;
    for chunk in 0..100 {
        let rows = (0..100).map(|idx| {
            let id = chunk * 100 + idx;
            (ColumnFamily::Base, key("h5", id), value(id, 64))
        });
        vault.write_cf_batch(rows).map_err(err)?;
    }
    let before_flush = status(&vault, &vault_dir)?;
    vault.flush().map_err(err)?;
    let seq = vault.latest_seq();
    drop(vault);
    let mut wal = Wal::open(vault_dir.join("wal"), wal_options).map_err(err)?;
    let recycler = WalRecycler::with_limits(200, 200, Duration::ZERO);
    let recycle = recycler.run_once_at(&mut wal, seq, 1);
    let inventory = wal.segment_inventory().map_err(err)?;
    drop(wal);
    let reopened = open_vault(
        &vault_dir,
        START_TS + 501,
        b"ph59-h5",
        FSV_MEMTABLE_BYTES,
        Some(wal_options),
    )?;
    let readback = count_readable(&reopened, "h5", 0, 10_000)?;
    let bounded = recycle.wal_bytes_active_after <= max_segment * 2;
    let passed = before_flush.wal.bytes > max_segment
        && recycle.triggered
        && recycle.wal_bytes_active_after < recycle.wal_bytes_active_before
        && bounded
        && readback == 10_000;
    Ok((
        passed,
        json!({
            "trigger": "10k durable rows held before flush, then flushed, WAL recycled, vault reopened",
            "expected": {"wal_grows_before_flush": true, "wal_bounded_post_recovery": true, "all_acked_present": 10000},
            "actual": {
                "status_before_flush": before_flush, "recycle": wal_result_json(&recycle),
                "wal_bounded_lte_bytes": max_segment * 2, "readback_rows_after_reopen": readback,
                "segment_inventory_after": inventory.iter().map(segment_json).collect::<Vec<_>>(),
                "panic_free": true
            },
            "metrics_text": recycle.to_metrics_text("ph59-h5")
        }),
    ))
}

fn panic_text(payload: Box<dyn std::any::Any + Send>) -> String {
    if let Some(text) = payload.downcast_ref::<&str>() {
        (*text).to_string()
    } else if let Some(text) = payload.downcast_ref::<String>() {
        text.clone()
    } else {
        "non-string panic payload".to_string()
    }
}
