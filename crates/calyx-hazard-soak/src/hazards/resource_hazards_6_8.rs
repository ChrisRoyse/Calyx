use calyx_aster::cf::{CfRouter, ColumnFamily};
use calyx_aster::gc::GcRateLimit;
use calyx_aster::mvcc::{Freshness, VersionedCfStore};
use calyx_core::Clock;
use calyx_forge::{
    AdmissionController, BlockDeallocator, DevicePtr, GpuBlockRegistry, Result as ForgeResult,
    VramBudgeter, VramProbe,
};
use serde::Serialize;
use serde_json::json;
use std::fs;
use std::path::Path;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use super::heap_soak::probe_h8_heap_oom;
use super::resource::{HazardResult, ProbeResult, run_probe};
use super::resource_support::{case_dir, err};

const START_TS: u64 = 1_800_100_000_000;
const FSV_MEMTABLE_BYTES: usize = 64 * 1024 * 1024;
const MIB: usize = 1024 * 1024;
const GIB: usize = 1024 * MIB;
const VRAM_BUDGET_CODE: &str = "CALYX_FORGE_VRAM_BUDGET";

pub fn run_hazards_6_8(root: &Path) -> Vec<HazardResult> {
    [
        (
            6,
            "MVCC version pile-up / long reader",
            probe_h6_long_reader as fn(&Path) -> ProbeResult,
        ),
        (7, "VRAM OOM admission", probe_h7_vram_oom),
        (8, "heap OOM bounded soak", probe_h8_heap_oom),
    ]
    .into_iter()
    .map(|(id, name, probe)| run_probe(root, id, name, probe))
    .collect()
}

fn probe_h6_long_reader(root: &Path) -> ProbeResult {
    let dir = case_dir(root, "h6_mvcc_long_reader")?;
    let vault_dir = dir.join("vault");
    let clock = calyx_core::FixedClock::new(START_TS + 600);
    let expired_clock = calyx_core::FixedClock::new(START_TS + 850);
    let router = CfRouter::open(&vault_dir, FSV_MEMTABLE_BYTES).map_err(err)?;
    let store = VersionedCfStore::new_with_router(0, router);
    let key = b"ph59-h6-long-reader-key".to_vec();

    write_versions(&store, &key, 1, 10_000, 512)?;
    let pinned = store.pin_snapshot_at(5_000, Freshness::FreshDerived, &clock, 200);
    let pinned_read_len = store
        .read_at(pinned, ColumnFamily::Base, &key, &clock)
        .map_err(err)?
        .map(|value| value.len())
        .unwrap_or(0);
    write_versions(&store, &key, 10_001, 20_000, 512)?;
    store.flush_all_cfs().map_err(err)?;

    let before_tick = store.snapshot_gc_tick(&clock, 1_000);
    let metrics_before = store.snapshot_gc_metrics(clock.now());
    let sst_bytes_before = tree_bytes(&vault_dir.join("cf"));
    let expired_error_code = match store.read_at(pinned, ColumnFamily::Base, &key, &expired_clock) {
        Ok(_) => "NO_ERROR".to_string(),
        Err(error) => error.code.to_string(),
    };
    let abort_tick = store.snapshot_gc_tick(&expired_clock, 1_000);
    store.set_snapshot_gc_rate_limit(GcRateLimit::new(25_000, Duration::ZERO));
    let gc_result = store
        .snapshot_version_gc_tick(&expired_clock)
        .map_err(err)?;
    store.flush_all_cfs().map_err(err)?;
    let after_tick = store.snapshot_gc_tick(&expired_clock, 1_000);
    let metrics_after = store.snapshot_gc_metrics(expired_clock.now());
    let sst_bytes_after = tree_bytes(&vault_dir.join("cf"));

    let passed = pinned_read_len == 512
        && before_tick.metrics.oldest_pinned_seq_gap >= 9_999
        && expired_error_code == "CALYX_READER_LEASE_EXPIRED"
        && abort_tick.metrics.reader_lease_expired_total == 1
        && after_tick.metrics.oldest_pinned_seq_gap < 10
        && gc_result.versions_reclaimed > 0
        && metrics_after.bytes_freed_total > metrics_before.bytes_freed_total
        && sst_bytes_after <= sst_bytes_before;

    Ok((
        passed,
        json!({
            "trigger": "reader pinned at seq 5000, 10000 newer versions, lease expiry, snapshot GC",
            "expected": {
                "oldest_pinned_seq_gap_gte": 9999,
                "expired_error_code": "CALYX_READER_LEASE_EXPIRED",
                "reader_lease_expired_total": 1,
                "post_gc_gap_lt": 10,
                "gc_bytes_delta_gt_zero": true,
                "disk_flat_or_improved": true
            },
            "actual": {
                "pinned_read_len": pinned_read_len,
                "before_tick": before_tick,
                "expired_error_code": expired_error_code,
                "abort_tick": abort_tick,
                "after_tick": after_tick,
                "gc_result": gc_result,
                "gc_bytes_before": metrics_before.bytes_freed_total,
                "gc_bytes_after": metrics_after.bytes_freed_total,
                "sst_bytes_before": sst_bytes_before,
                "sst_bytes_after": sst_bytes_after,
                "metrics_before": metrics_before,
                "metrics_after": metrics_after,
                "panic_free": true
            },
            "metrics_text": format!(
                "calyx_reader_lease_expired_total{{vault=\"ph59-h6\"}} {}\ncalyx_oldest_pinned_seq_gap{{vault=\"ph59-h6\"}} {}\ncalyx_snapshot_gc_bytes_freed_total{{vault=\"ph59-h6\"}} {}\n",
                abort_tick.metrics.reader_lease_expired_total,
                after_tick.metrics.oldest_pinned_seq_gap,
                metrics_after.bytes_freed_total
            )
        }),
    ))
}

fn probe_h7_vram_oom(root: &Path) -> ProbeResult {
    let _dir = case_dir(root, "h7_vram_oom")?;
    let budgeter = VramBudgeter::with_soft_cap(2 * GIB, StaticProbe { free: 64 * GIB });
    let registry = GpuBlockRegistry::new(&budgeter, NoopDealloc, 16);
    let controller = AdmissionController::new(&budgeter, Arc::new(Mutex::new(registry)), 0, 1);
    let before = budgeter.stats();
    let nvidia_before = query_nvidia_smi();
    let outcomes = run_vram_dispatches(&controller);
    let after = budgeter.stats();
    let nvidia_after = query_nvidia_smi();
    let zero_budget_error = zero_budget_error_code();
    let oom_lines = command_text("sh", &["-lc", "dmesg 2>/dev/null | grep -i oom || true"]);
    let max_memory_delta_mib = nvidia_before
        .memory_used_mib
        .zip(nvidia_after.memory_used_mib)
        .map(|(before, after)| after.saturating_sub(before))
        .unwrap_or(0);
    let nvidia_ok = nvidia_before.ok && nvidia_after.ok;
    let passed = outcomes.budget_errors >= 1
        && outcomes.panics == 0
        && outcomes.other_errors == 0
        && after.failed_total >= 1
        && zero_budget_error == VRAM_BUDGET_CODE
        && nvidia_ok
        && max_memory_delta_mib <= 2_560
        && oom_lines.trim().is_empty();

    Ok((
        passed,
        json!({
            "trigger": "20 concurrent 200MiB Forge admissions against a 2GiB soft cap",
            "expected": {
                "budget_errors_gte": 1,
                "error_code": VRAM_BUDGET_CODE,
                "nvidia_delta_mib_lte": 2560,
                "dmesg_oom_lines": ""
            },
            "actual": {
                "dispatches": outcomes,
                "before": before,
                "after": after,
                "zero_budget_error_code": zero_budget_error,
                "nvidia_smi_before": nvidia_before,
                "nvidia_smi_after": nvidia_after,
                "max_memory_delta_mib": max_memory_delta_mib,
                "dmesg_oom_lines": oom_lines,
                "panic_free": true
            },
            "metrics_text": after.admission_metrics_text()
                + &format!("calyx_forge_vram_max_memory_delta_mib{{vault=\"ph59-h7\"}} {max_memory_delta_mib}\n")
        }),
    ))
}

fn write_versions(
    store: &VersionedCfStore,
    key: &[u8],
    start: u64,
    end: u64,
    value_len: usize,
) -> Result<(), String> {
    for seq in start..=end {
        let committed = store
            .commit_batch([(
                ColumnFamily::Base,
                key.to_vec(),
                known_value(seq, value_len),
            )])
            .map_err(err)?;
        if committed != seq {
            return Err(format!("expected seq {seq}, got {committed}"));
        }
    }
    Ok(())
}

fn known_value(seq: u64, len: usize) -> Vec<u8> {
    let mut value = format!("ph59-h6-v{seq:05}:").into_bytes();
    value.resize(len, b'x');
    value
}

fn tree_bytes(path: &Path) -> u64 {
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return 0;
    };
    if metadata.is_file() {
        return metadata.len();
    }
    if metadata.is_dir() {
        return fs::read_dir(path)
            .map(|entries| {
                entries
                    .filter_map(std::result::Result::ok)
                    .map(|entry| tree_bytes(&entry.path()))
                    .sum()
            })
            .unwrap_or(0);
    }
    metadata.len()
}

#[derive(Clone, Copy)]
struct StaticProbe {
    free: usize,
}

impl VramProbe for StaticProbe {
    fn free_device_vram(&self) -> ForgeResult<usize> {
        Ok(self.free)
    }
}

#[derive(Clone, Default)]
struct NoopDealloc;

impl BlockDeallocator for NoopDealloc {
    fn free(&self, _ptr: DevicePtr, _size: usize) -> ForgeResult<()> {
        Ok(())
    }
}

#[derive(Default, Serialize)]
struct VramDispatchReadback {
    requested: usize,
    success: usize,
    budget_errors: usize,
    panics: usize,
    other_errors: usize,
}

fn run_vram_dispatches(
    controller: &AdmissionController<'_, StaticProbe, NoopDealloc>,
) -> VramDispatchReadback {
    thread::scope(|scope| {
        let handles = (0..20)
            .map(|_| {
                scope.spawn(|| {
                    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        let deadline = Instant::now() + Duration::from_secs(2);
                        controller.run_with_admission(200 * MIB, 1, deadline, |offset, len| {
                            thread::sleep(Duration::from_millis(150));
                            let _range_readback = (offset, len);
                            Ok(())
                        })
                    }));
                    classify_vram_result(outcome)
                })
            })
            .collect::<Vec<_>>();
        let mut readback = VramDispatchReadback {
            requested: 20,
            ..VramDispatchReadback::default()
        };
        for handle in handles {
            match handle.join().unwrap_or(VramOutcome::Panic) {
                VramOutcome::Success => readback.success += 1,
                VramOutcome::Budget => readback.budget_errors += 1,
                VramOutcome::Other => readback.other_errors += 1,
                VramOutcome::Panic => readback.panics += 1,
            }
        }
        readback
    })
}

enum VramOutcome {
    Success,
    Budget,
    Other,
    Panic,
}

fn classify_vram_result(outcome: std::thread::Result<ForgeResult<()>>) -> VramOutcome {
    match outcome {
        Err(_) => VramOutcome::Panic,
        Ok(Ok(_)) => VramOutcome::Success,
        Ok(Err(error)) if error.code() == VRAM_BUDGET_CODE => VramOutcome::Budget,
        Ok(Err(_)) => VramOutcome::Other,
    }
}

fn zero_budget_error_code() -> String {
    let budgeter = VramBudgeter::with_soft_cap(0, StaticProbe { free: 64 * GIB });
    let registry = GpuBlockRegistry::new(&budgeter, NoopDealloc, 1);
    let controller = AdmissionController::new(&budgeter, Arc::new(Mutex::new(registry)), 0, 1);
    controller
        .run_with_admission(
            1,
            1,
            Instant::now() + Duration::from_secs(1),
            |_offset, _len| Ok(()),
        )
        .expect_err("zero VRAM budget must fail closed")
        .code()
        .to_string()
}

#[derive(Serialize)]
struct NvidiaReadback {
    ok: bool,
    memory_used_mib: Option<u64>,
    memory_free_mib: Option<u64>,
    raw: String,
}

fn query_nvidia_smi() -> NvidiaReadback {
    let output = Command::new("nvidia-smi")
        .args([
            "--query-gpu=memory.used,memory.free",
            "--format=csv,noheader,nounits",
        ])
        .output();
    let Ok(output) = output else {
        return NvidiaReadback {
            ok: false,
            memory_used_mib: None,
            memory_free_mib: None,
            raw: "nvidia-smi unavailable".to_string(),
        };
    };
    let raw = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let mut parts = raw.lines().next().unwrap_or("").split(',').map(str::trim);
    NvidiaReadback {
        ok: output.status.success(),
        memory_used_mib: parts.next().and_then(|value| value.parse().ok()),
        memory_free_mib: parts.next().and_then(|value| value.parse().ok()),
        raw,
    }
}

fn command_text(command: &str, args: &[&str]) -> String {
    Command::new(command)
        .args(args)
        .output()
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .unwrap_or_else(|error| format!("command unavailable: {error}"))
}
