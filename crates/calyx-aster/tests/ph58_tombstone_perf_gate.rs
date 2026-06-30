#![cfg(target_os = "linux")]

mod fsv_support;
#[path = "soak_ph58/serving.rs"]
mod ph58_serving;

use calyx_aster::cf::ColumnFamily;
use calyx_aster::gc::{
    CompactionGcReclaimer, GcRateLimit, TombstoneInventory, VaultCompactionGcTarget,
    scan_tombstone_inventory,
};
use calyx_aster::mvcc::tombstone_value;
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{Clock, Ts, VaultId};
use serde_json::{Value, json};
use std::fs;
use std::hint::black_box;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use fsv_support::{reset_dir, write_blake3_sums, write_json};
use ph58_serving::live_base_readback_count;

const START_TS: Ts = 1_800_000_800_000;
const FSV_MEMTABLE_BYTES: usize = 64 * 1024 * 1024;
const WRITE_BATCH: usize = 1_000;
const LIVE_END: u64 = 50_000;
const TOMBSTONE_END: u64 = 30_000;
const READ_START: u64 = 30_000;
const READ_END: u64 = 32_000;
const MEASUREMENT_ROUNDS: usize = 5;
const DEFAULT_MAX_P99_RATIO: f64 = 2.0;
const DEFAULT_MAX_LOAD_PER_CPU: f64 = 1.0;

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
}

impl Clock for SharedClock {
    fn now(&self) -> Ts {
        self.now.load(Ordering::Relaxed)
    }
}

#[derive(Clone, Copy, Debug)]
struct HostLoad {
    load1: f64,
    load5: f64,
    load15: f64,
    cpu_count: usize,
}

impl HostLoad {
    fn load1_per_cpu(self) -> f64 {
        self.load1 / self.cpu_count as f64
    }

    fn to_json(self) -> Value {
        json!({
            "load1": self.load1,
            "load5": self.load5,
            "load15": self.load15,
            "cpu_count": self.cpu_count,
            "load1_per_cpu": self.load1_per_cpu(),
        })
    }
}

#[derive(Clone, Debug)]
struct GateConfig {
    max_p99_ratio: f64,
    max_load_per_cpu: f64,
}

#[test]
#[ignore = "serial PH58 performance gate; run scripts/ph58_tombstone_perf_gate.sh"]
fn ph58_tombstone_serving_p99_serial_gate() {
    require_explicit_gate();
    let config = GateConfig {
        max_p99_ratio: env_f64("CALYX_PH58_MAX_P99_RATIO", DEFAULT_MAX_P99_RATIO, 1.0),
        max_load_per_cpu: env_f64(
            "CALYX_PH58_MAX_HOST_LOAD_PER_CPU",
            DEFAULT_MAX_LOAD_PER_CPU,
            0.01,
        ),
    };
    let root = required_fsv_root();
    fs::create_dir_all(&root).expect("create PH58 perf FSV root");
    let load_before = controlled_load(&root, "before", config.max_load_per_cpu);
    let work = root.join("work");
    reset_dir(&work);

    let started = Instant::now();
    let vault_dir = work.join("vault");
    let clock = SharedClock::new(START_TS);
    let vault = open_vault(&vault_dir, clock, b"issue1048-ph58-tombstone-perf");
    write_live_range(&vault, 0, LIVE_END);
    vault.flush().expect("flush live rows");
    write_tombstone_range(&vault, 0, TOMBSTONE_END);
    vault.flush().expect("flush tombstone rows");
    vault
        .snapshot_version_gc_once(GcRateLimit::new(100_000, Duration::ZERO))
        .expect("snapshot GC before tombstone perf gate");
    vault.flush().expect("flush after snapshot GC");

    let before_inventory = scan_tombstone_inventory(&vault_dir).expect("scan tombstones before");
    assert!(
        before_inventory.tombstone_ratio() > 0.5,
        "CALYX_PH58_FIXTURE_INVALID before tombstone ratio {} <= 0.5",
        before_inventory.tombstone_ratio()
    );
    let before_readback = live_base_readback_count(&vault, READ_START, READ_END);
    assert_eq!(before_readback.visible, (READ_END - READ_START) as usize);
    assert_eq!(before_readback.missing, 0);
    let baseline = measure_read_p99_ns(&vault, READ_START, READ_END, MEASUREMENT_ROUNDS);

    let reclaimer = CompactionGcReclaimer::with_limits(0.2, 1, 1_000_000_000, 0);
    let target = VaultCompactionGcTarget {
        vault: &vault,
        vault_dir: &vault_dir,
    };
    let mut compaction_results = Vec::new();
    for pass in 0..3 {
        compaction_results.push(reclaimer.maybe_trigger_at(&target, 0.5, pass * 1_000));
    }

    let after_inventory = scan_tombstone_inventory(&vault_dir).expect("scan tombstones after");
    assert!(
        after_inventory.tombstone_ratio() <= 0.1,
        "CALYX_PH58_TOMBSTONE_GC_INCOMPLETE after tombstone ratio {} > 0.1",
        after_inventory.tombstone_ratio()
    );
    let after_readback = live_base_readback_count(&vault, READ_START, READ_END);
    assert_eq!(after_readback.visible, (READ_END - READ_START) as usize);
    assert_eq!(after_readback.missing, 0);
    let after = measure_read_p99_ns(&vault, READ_START, READ_END, MEASUREMENT_ROUNDS);
    let allowed_after_p99_ns = ((baseline.p99_ns as f64) * config.max_p99_ratio).ceil() as u64;
    let gate_passed = after.p99_ns <= allowed_after_p99_ns;
    let load_after = controlled_load(&root, "after", config.max_load_per_cpu);

    let summary = json!({
        "issue": 1048,
        "gate": "ph58_tombstone_serving_p99_serial_gate",
        "source_of_truth": "physical Aster vault SST/WAL rows under work/vault, scan_tombstone_inventory readback, live Base CF readback, and this JSON artifact read back from disk",
        "expected": {
            "explicit_gate_env": "CALYX_PH58_TOMBSTONE_PERF_GATE=1",
            "serial_runner": "scripts/ph58_tombstone_perf_gate.sh uses cargo test --test-threads=1",
            "tombstone_ratio_before_gt_0_5": true,
            "tombstone_ratio_after_lte_0_1": true,
            "live_readback_visible": READ_END - READ_START,
            "live_readback_missing": 0,
            "after_p99_ns_lte": allowed_after_p99_ns,
            "max_p99_ratio": config.max_p99_ratio,
            "max_host_load_per_cpu": config.max_load_per_cpu,
        },
        "actual": {
            "gate_passed": gate_passed,
            "elapsed_ms": started.elapsed().as_millis() as u64,
            "baseline": baseline.to_json(),
            "after": after.to_json(),
            "p99_ratio": (after.p99_ns as f64) / (baseline.p99_ns.max(1) as f64),
            "allowed_after_p99_ns": allowed_after_p99_ns,
            "before_inventory": inventory_json(&before_inventory),
            "after_inventory": inventory_json(&after_inventory),
            "before_readback": before_readback.to_json(),
            "after_readback": after_readback.to_json(),
            "compaction_results": compaction_results.into_iter().map(compaction_result_json).collect::<Vec<_>>(),
        },
        "host_context": {
            "load_before": load_before.to_json(),
            "load_after": load_after.to_json(),
            "cpu_model": first_cpu_model(),
            "uname": command_text("uname", &["-a"]),
            "df_vault": command_text("df", &["-B1", vault_dir.to_str().expect("vault path utf8")]),
            "vault_dir": vault_dir.display().to_string(),
            "git_head": git_text(&["rev-parse", "--verify", "HEAD"]),
            "git_status_short": git_text(&["status", "--short", "--branch"]),
        }
    });
    write_json(&root.join("ph58_tombstone_perf_gate.json"), &summary);
    write_blake3_sums(&root);
    let readback: Value = serde_json::from_slice(
        &fs::read(root.join("ph58_tombstone_perf_gate.json")).expect("read summary artifact"),
    )
    .expect("decode summary artifact");
    assert_eq!(readback["actual"]["gate_passed"], json!(true));
    assert!(
        gate_passed,
        "CALYX_PH58_TOMBSTONE_P99_REGRESSION baseline_p99_ns={} after_p99_ns={} allowed_after_p99_ns={} max_ratio={}",
        baseline.p99_ns, after.p99_ns, allowed_after_p99_ns, config.max_p99_ratio
    );
    println!("PH58_TOMBSTONE_PERF_FSV_ROOT={}", root.display());
    println!(
        "PH58_TOMBSTONE_PERF_SUMMARY={}",
        root.join("ph58_tombstone_perf_gate.json").display()
    );
}

#[derive(Clone, Debug)]
struct P99Measurement {
    samples: usize,
    rounds: Vec<u64>,
    p99_ns: u64,
}

impl P99Measurement {
    fn to_json(&self) -> Value {
        json!({
            "sample_count": self.samples,
            "round_p99_ns": self.rounds,
            "p99_ns": self.p99_ns,
        })
    }
}

fn measure_read_p99_ns(
    vault: &AsterVault<SharedClock>,
    start: u64,
    end: u64,
    rounds: usize,
) -> P99Measurement {
    let snapshot = vault.latest_seq();
    let mut all_samples = Vec::with_capacity((end - start) as usize * rounds);
    let mut round_p99 = Vec::with_capacity(rounds);
    for _ in 0..rounds {
        let mut samples = Vec::with_capacity((end - start) as usize);
        for id in start..end {
            let key = format!("key-{id:05}");
            let started = Instant::now();
            let value = vault
                .read_cf_at(snapshot, ColumnFamily::Base, key.as_bytes())
                .expect("serving read")
                .expect("live row");
            black_box(value.len());
            samples.push(ns_u64(started.elapsed()));
        }
        samples.sort_unstable();
        round_p99.push(percentile_99(&samples));
        all_samples.extend(samples);
    }
    all_samples.sort_unstable();
    P99Measurement {
        samples: all_samples.len(),
        rounds: round_p99,
        p99_ns: percentile_99(&all_samples),
    }
}

fn percentile_99(samples: &[u64]) -> u64 {
    assert!(!samples.is_empty(), "CALYX_PH58_NO_P99_SAMPLES");
    samples[(samples.len() * 99 / 100).min(samples.len() - 1)]
}

fn ns_u64(duration: Duration) -> u64 {
    u64::try_from(duration.as_nanos().max(1)).unwrap_or(u64::MAX)
}

fn write_live_range(vault: &AsterVault<SharedClock>, start: u64, end: u64) {
    let mut next = start;
    while next < end {
        let upper = (next + WRITE_BATCH as u64).min(end);
        let rows = (next..upper).map(|id| {
            (
                ColumnFamily::Base,
                format!("key-{id:05}").into_bytes(),
                format!("value-{id:05}").into_bytes(),
            )
        });
        vault.write_cf_batch(rows).expect("write live batch");
        next = upper;
    }
}

fn write_tombstone_range(vault: &AsterVault<SharedClock>, start: u64, end: u64) {
    let mut next = start;
    while next < end {
        let upper = (next + WRITE_BATCH as u64).min(end);
        let rows = (next..upper).map(|id| {
            (
                ColumnFamily::Base,
                format!("key-{id:05}").into_bytes(),
                tombstone_value(),
            )
        });
        vault.write_cf_batch(rows).expect("write tombstone batch");
        next = upper;
    }
}

fn open_vault(
    vault_dir: &Path,
    clock: SharedClock,
    encryption_key: &[u8],
) -> AsterVault<SharedClock> {
    fs::create_dir_all(vault_dir).expect("create vault dir");
    AsterVault::new_durable_with_clock(
        vault_dir,
        vault_id(),
        encryption_key.to_vec(),
        VaultOptions {
            memtable_byte_cap: FSV_MEMTABLE_BYTES,
            ..VaultOptions::default()
        },
        clock,
    )
    .expect("open durable vault")
}

fn controlled_load(root: &Path, phase: &str, max_per_cpu: f64) -> HostLoad {
    let load = host_load();
    if load.load1_per_cpu() > max_per_cpu {
        let refusal = json!({
            "error_code": "CALYX_PH58_HOST_LOAD_UNCONTROLLED",
            "phase": phase,
            "max_load_per_cpu": max_per_cpu,
            "load": load.to_json(),
            "remediation": "rerun scripts/ph58_tombstone_perf_gate.sh when no competing build/test workload is active",
        });
        write_json(
            &root.join(format!("ph58_tombstone_perf_gate_{phase}_refusal.json")),
            &refusal,
        );
        panic!(
            "CALYX_PH58_HOST_LOAD_UNCONTROLLED phase={phase} load1_per_cpu={} max_load_per_cpu={max_per_cpu}",
            load.load1_per_cpu()
        );
    }
    load
}

fn host_load() -> HostLoad {
    let cpu_count = std::thread::available_parallelism()
        .expect("CALYX_PH58_CPU_COUNT_UNAVAILABLE")
        .get();
    let loadavg = fs::read_to_string("/proc/loadavg").expect("CALYX_PH58_LOADAVG_UNAVAILABLE");
    let parts = loadavg.split_whitespace().take(3).collect::<Vec<_>>();
    if parts.len() != 3 {
        panic!("CALYX_PH58_LOADAVG_INVALID raw={loadavg:?}");
    }
    HostLoad {
        load1: parse_load(parts[0], "load1"),
        load5: parse_load(parts[1], "load5"),
        load15: parse_load(parts[2], "load15"),
        cpu_count,
    }
}

fn parse_load(value: &str, field: &str) -> f64 {
    let parsed = value.parse::<f64>().unwrap_or_else(|_| {
        panic!("CALYX_PH58_LOADAVG_INVALID field={field} value={value}");
    });
    if !parsed.is_finite() || parsed < 0.0 {
        panic!("CALYX_PH58_LOADAVG_INVALID field={field} value={value}");
    }
    parsed
}

fn env_f64(name: &str, default: f64, min: f64) -> f64 {
    let value = match std::env::var(name) {
        Ok(raw) => raw.parse::<f64>().unwrap_or_else(|_| {
            panic!("CALYX_PH58_INVALID_CONFIG env={name} value={raw}");
        }),
        Err(_) => default,
    };
    if !value.is_finite() || value < min {
        panic!("CALYX_PH58_INVALID_CONFIG env={name} value={value} min={min}");
    }
    value
}

fn require_explicit_gate() {
    match std::env::var("CALYX_PH58_TOMBSTONE_PERF_GATE") {
        Ok(value) if value == "1" => {}
        Ok(value) => panic!("CALYX_PH58_PERF_GATE_NOT_EXPLICIT value={value}"),
        Err(_) => {
            panic!("CALYX_PH58_PERF_GATE_NOT_EXPLICIT missing=CALYX_PH58_TOMBSTONE_PERF_GATE")
        }
    }
}

fn required_fsv_root() -> PathBuf {
    std::env::var_os("CALYX_FSV_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| panic!("CALYX_PH58_FSV_ROOT_REQUIRED env=CALYX_FSV_ROOT"))
}

fn inventory_json(inventory: &TombstoneInventory) -> Value {
    json!({
        "tombstone_keys": inventory.tombstone_keys(),
        "live_keys": inventory.live_keys(),
        "tombstone_ratio": inventory.tombstone_ratio(),
        "total_sst_bytes": inventory.total_sst_bytes(),
        "per_cf": inventory.per_cf.iter().map(|cf| json!({
            "cf": cf.cf_name,
            "sst_files": cf.sst_files,
            "sst_bytes": cf.sst_bytes,
            "live_keys": cf.live_keys,
            "tombstone_keys": cf.tombstone_keys,
            "tombstone_ratio": cf.tombstone_ratio(),
        })).collect::<Vec<_>>()
    })
}

fn compaction_result_json(result: calyx_aster::gc::CompactionGcResult) -> Value {
    json!({
        "triggered": result.triggered,
        "rate_limited": result.rate_limited,
        "skipped_reason": result.skipped_reason,
        "error_code": result.error_code,
        "tombstone_ratio_before": result.tombstone_ratio_before,
        "tombstone_ratio_after": result.tombstone_ratio_after,
        "bytes_compacted": result.bytes_compacted,
        "bytes_freed": result.bytes_freed,
        "tombstones_removed": result.tombstones_removed,
        "compaction_debt": result.compaction_debt,
        "compacted_cfs": result.compacted_cfs,
    })
}

fn first_cpu_model() -> String {
    let info = fs::read_to_string("/proc/cpuinfo").expect("read /proc/cpuinfo");
    info.lines()
        .find_map(|line| {
            line.strip_prefix("model name")
                .and_then(|rest| rest.split_once(':'))
                .map(|(_, value)| value.trim().to_string())
        })
        .unwrap_or_else(|| "unknown".to_string())
}

fn command_text(command: &str, args: &[&str]) -> String {
    let output = Command::new(command)
        .args(args)
        .output()
        .unwrap_or_else(|err| panic!("CALYX_PH58_COMMAND_FAILED command={command} error={err}"));
    assert!(
        output.status.success(),
        "CALYX_PH58_COMMAND_FAILED command={} stderr={}",
        command,
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("stdout utf8")
}

fn git_text(args: &[&str]) -> String {
    let output = Command::new("git")
        .current_dir(repo_root())
        .args(args)
        .output()
        .expect("run git");
    assert!(
        output.status.success(),
        "CALYX_PH58_GIT_COMMAND_FAILED args={args:?} stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("git stdout utf8")
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("repo root")
        .to_path_buf()
}

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FBG".parse().expect("vault id")
}
