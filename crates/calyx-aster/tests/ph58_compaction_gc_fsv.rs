#![cfg(target_os = "linux")]

//! PH58 T03 manual FSV driver for compaction GC tombstone reclaim.

use calyx_aster::cf::ColumnFamily;
use calyx_aster::gc::{
    CompactionGcReclaimer, TombstoneInventory, VaultCompactionGcTarget, scan_tombstone_inventory,
};
use calyx_aster::mvcc::tombstone_value;
use calyx_aster::resource::{ResourceStatus, VramBudgetStatus};
use calyx_aster::sst::SstReader;
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{Clock, Ts, VaultId};
use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};
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
}

impl Clock for SharedClock {
    fn now(&self) -> Ts {
        self.now.load(Ordering::Relaxed)
    }
}

#[test]
#[ignore = "manual FSV writes PH58 compaction-GC source-of-truth artifacts"]
fn ph58_compaction_gc_tombstone_fsv() {
    let root = fsv_root_os("CALYX_FSV_ROOT", "calyx-ph58-compaction-gc");
    reset_dir(&root);
    fs::write(
        root.join("cleanup-tag.txt"),
        b"issue483 PH58 compaction GC tombstone synthetic FSV data\n",
    )
    .expect("write cleanup tag");

    let happy = run_happy_path(&root.join("happy"));
    let low_io = run_low_io_edge(&root.join("edge-low-io"));
    let zero = run_zero_tombstone_edge(&root.join("edge-zero-tombstone"));
    let corrupt = run_corrupt_sst_edge(&root.join("edge-corrupt-sst"));
    let summary = json!({
        "issue": 483,
        "trigger": "CompactionGcReclaimer maybe_trigger after known tombstone-heavy SST bytes",
        "outcome": "MVCC tombstone SST rows are pruned by real Aster compaction while live rows remain readable",
        "happy": happy,
        "low_io_edge": low_io,
        "zero_tombstone_edge": zero,
        "corrupt_sst_edge": corrupt
    });
    fs::write(
        root.join("ph58-compaction-gc-readback.json"),
        serde_json::to_vec_pretty(&summary).expect("encode summary"),
    )
    .expect("write summary");
    println!(
        "PH58 compaction GC FSV: before_ratio={} after_ratio={} tombstones_removed={} corrupt_code={}",
        summary["happy"]["actual"]["result"]["tombstone_ratio_before"],
        summary["happy"]["actual"]["result"]["tombstone_ratio_after"],
        summary["happy"]["actual"]["result"]["tombstones_removed"],
        summary["corrupt_sst_edge"]["actual"]["error_code"]
    );
}

fn run_happy_path(root: &Path) -> Value {
    reset_dir(root);
    let vault_dir = root.join("vault");
    let vault = open_vault(&vault_dir);
    write_fixture(&vault, 40, 60);
    vault.flush().expect("flush before compaction GC");
    let before = scan_tombstone_inventory(&vault_dir).expect("scan before");
    write_readbacks(root, "before", &vault, &vault_dir, None);

    let reclaimer = CompactionGcReclaimer::with_limits(0.2, 1, 1_000_000, 0);
    let target = VaultCompactionGcTarget {
        vault: &vault,
        vault_dir: &vault_dir,
    };
    let result = reclaimer.maybe_trigger_at(&target, 0.5, 0);
    let after = scan_tombstone_inventory(&vault_dir).expect("scan after");
    write_readbacks(
        root,
        "after",
        &vault,
        &vault_dir,
        Some(result.to_metrics_text("issue483")),
    );
    assert!(result.triggered);
    let before_base = cf_stats(&before, ColumnFamily::Base);
    let after_base = cf_stats(&after, ColumnFamily::Base);
    assert_eq!(before_base.tombstone_keys, 120);
    assert_eq!(before_base.live_keys, 80);
    assert_eq!(after.tombstone_keys(), 0);
    assert_eq!(after_base.live_keys, 40);
    assert_eq!(result.tombstones_removed, 120);
    assert_eq!(result.compacted_cfs, vec!["base"]);
    assert_eq!(read_live_count(&vault, 40), 40);
    json!({
        "input": {"live_rows": 40, "tombstone_rows": 60, "disk_io_available_fraction": 0.5},
        "expected": {
            "base_tombstone_ratio_before_gt_0_5": true,
            "physical_tombstone_sst_rows_removed": 120,
            "ratio_after": 0.0,
            "live_rows_readable": 40
        },
        "actual": {"before": inventory_json(&before), "after": inventory_json(&after), "result": result_json(&result)}
    })
}

fn run_low_io_edge(root: &Path) -> Value {
    reset_dir(root);
    let vault_dir = root.join("vault");
    let vault = open_vault(&vault_dir);
    write_fixture(&vault, 40, 60);
    vault.flush().expect("flush low-io fixture");
    let before = scan_tombstone_inventory(&vault_dir).expect("scan low-io before");
    write_readbacks(root, "before", &vault, &vault_dir, None);
    let reclaimer = CompactionGcReclaimer::with_limits(0.2, 1, 1_000_000, 0);
    let target = VaultCompactionGcTarget {
        vault: &vault,
        vault_dir: &vault_dir,
    };
    let result = reclaimer.maybe_trigger_at(&target, 0.1, 0);
    let after = scan_tombstone_inventory(&vault_dir).expect("scan low-io after");
    write_readbacks(
        root,
        "after",
        &vault,
        &vault_dir,
        Some(result.to_metrics_text("issue483-low-io")),
    );
    assert!(!result.triggered);
    assert_eq!(before.tombstone_keys(), after.tombstone_keys());
    json!({
        "input": {"disk_io_available_fraction": 0.1, "max_io_fraction": 0.2},
        "expected": {"triggered": false, "tombstones_unchanged": true},
        "actual": {"before": inventory_json(&before), "after": inventory_json(&after), "result": result_json(&result)}
    })
}

fn run_zero_tombstone_edge(root: &Path) -> Value {
    reset_dir(root);
    let vault_dir = root.join("vault");
    let vault = open_vault(&vault_dir);
    write_fixture(&vault, 20, 0);
    vault.flush().expect("flush zero fixture");
    let before = scan_tombstone_inventory(&vault_dir).expect("scan zero before");
    write_readbacks(root, "before", &vault, &vault_dir, None);
    let reclaimer = CompactionGcReclaimer::with_limits(0.2, 1, 1_000_000, 0);
    let target = VaultCompactionGcTarget {
        vault: &vault,
        vault_dir: &vault_dir,
    };
    let result = reclaimer.maybe_trigger_at(&target, 0.5, 0);
    let after = scan_tombstone_inventory(&vault_dir).expect("scan zero after");
    write_readbacks(
        root,
        "after",
        &vault,
        &vault_dir,
        Some(result.to_metrics_text("issue483-zero-tombstone")),
    );
    assert!(!result.triggered);
    assert_eq!(before.tombstone_keys(), 0);
    assert_eq!(after.tombstone_keys(), 0);
    json!({
        "input": {"live_rows": 20, "tombstone_rows": 0},
        "expected": {"triggered": false, "ratio": 0.0},
        "actual": {"before": inventory_json(&before), "after": inventory_json(&after), "result": result_json(&result)}
    })
}

fn run_corrupt_sst_edge(root: &Path) -> Value {
    reset_dir(root);
    let vault_dir = root.join("vault");
    let vault = open_vault(&vault_dir);
    write_fixture(&vault, 4, 8);
    vault.flush().expect("flush corrupt fixture");
    let before_files = sorted_ssts(&vault_dir.join("cf/base"));
    let corrupt_path = before_files.first().expect("sst to corrupt");
    let mut bytes = fs::read(corrupt_path).expect("read sst to corrupt");
    bytes[0] ^= 0xff;
    fs::write(corrupt_path, bytes).expect("write corrupt sst");

    let reclaimer = CompactionGcReclaimer::with_limits(0.2, 1, 1_000_000, 0);
    let target = VaultCompactionGcTarget {
        vault: &vault,
        vault_dir: &vault_dir,
    };
    let result = reclaimer.maybe_trigger_at(&target, 0.5, 0);
    assert_eq!(result.error_code, Some("CALYX_ASTER_CORRUPT_SHARD"));
    json!({
        "input": {"corrupted_file": corrupt_path.display().to_string(), "mutation": "first byte xor 0xff"},
        "expected": {"error_code": "CALYX_ASTER_CORRUPT_SHARD", "triggered": false},
        "actual": {"result": result_json(&result), "error_code": result.error_code}
    })
}

fn write_fixture(vault: &AsterVault<SharedClock>, live_rows: u64, tombstone_rows: u64) {
    for id in 0..live_rows {
        vault
            .write_cf(
                ColumnFamily::Base,
                format!("live-{id:04}").into_bytes(),
                format!("value-{id:04}").into_bytes(),
            )
            .expect("write live");
    }
    vault.flush().expect("flush live rows");
    for id in 0..tombstone_rows {
        vault
            .write_cf(
                ColumnFamily::Base,
                format!("dead-{id:04}").into_bytes(),
                tombstone_value(),
            )
            .expect("write tombstone");
    }
    vault.flush().expect("flush tombstone rows");
}

fn read_live_count(vault: &AsterVault<SharedClock>, expected: u64) -> u64 {
    (0..expected)
        .filter(|id| {
            vault
                .read_cf_at(
                    vault.latest_seq(),
                    ColumnFamily::Base,
                    format!("live-{id:04}").as_bytes(),
                )
                .expect("read live")
                .is_some()
        })
        .count() as u64
}

fn open_vault(vault_dir: &Path) -> AsterVault<SharedClock> {
    fs::create_dir_all(vault_dir).expect("create vault dir");
    AsterVault::new_durable_with_clock(
        vault_dir,
        vault_id(),
        b"issue483-ph58-compaction-gc".to_vec(),
        VaultOptions::default(),
        SharedClock::new(10_000),
    )
    .expect("open durable vault")
}

fn write_readbacks(
    root: &Path,
    phase: &str,
    vault: &AsterVault<SharedClock>,
    vault_dir: &Path,
    metrics: Option<String>,
) {
    let inventory = scan_tombstone_inventory(vault_dir).expect("scan inventory");
    let status = status(vault, vault_dir);
    fs::write(
        root.join(format!("inventory-{phase}.json")),
        serde_json::to_vec_pretty(&inventory_json(&inventory)).expect("inventory json"),
    )
    .expect("write inventory");
    fs::write(
        root.join(format!("status-{phase}.json")),
        serde_json::to_vec_pretty(&status).expect("status json"),
    )
    .expect("write status");
    fs::write(
        root.join(format!("metrics-{phase}.prom")),
        metrics.unwrap_or_else(|| status.to_metrics_text("issue483")),
    )
    .expect("write metrics");
    fs::write(
        root.join(format!("base-cf-{phase}.txt")),
        dump_base_cf(vault_dir),
    )
    .expect("write base dump");
    let vault_path = vault_dir.to_str().expect("vault path utf8");
    fs::write(
        root.join(format!("du-{phase}.txt")),
        command_text("du", &["-sb", vault_path]),
    )
    .expect("write du");
    fs::write(
        root.join(format!("df-{phase}.txt")),
        command_text("df", &["-B1", vault_path]),
    )
    .expect("write df");
}

fn inventory_json(inventory: &TombstoneInventory) -> Value {
    json!({
        "tombstone_keys": inventory.tombstone_keys(),
        "live_keys": inventory.live_keys(),
        "tombstone_ratio": inventory.tombstone_ratio(),
        "total_sst_bytes": inventory.total_sst_bytes(),
        "write_amp": inventory.io_stats.write_amp(),
        "per_cf": inventory.per_cf.iter().map(|cf| json!({
            "cf": cf.cf_name,
            "sst_files": cf.sst_files,
            "sst_bytes": cf.sst_bytes,
            "live_keys": cf.live_keys,
            "tombstone_keys": cf.tombstone_keys,
            "tombstone_ratio": cf.tombstone_ratio()
        })).collect::<Vec<_>>()
    })
}

fn cf_stats(
    inventory: &TombstoneInventory,
    cf: ColumnFamily,
) -> &calyx_aster::gc::TombstoneCfStats {
    inventory
        .per_cf
        .iter()
        .find(|stats| stats.cf == cf)
        .expect("column family stats")
}

fn result_json(result: &calyx_aster::gc::CompactionGcResult) -> Value {
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
        "write_amp_before": result.write_amp_before,
        "write_amp_after": result.write_amp_after,
        "compaction_debt": result.compaction_debt,
        "compaction_debt_alert": result.compaction_debt_alert,
        "compacted_cfs": result.compacted_cfs
    })
}

fn dump_base_cf(vault_dir: &Path) -> String {
    let mut out = String::new();
    for path in sorted_ssts(&vault_dir.join("cf/base")) {
        let len = fs::metadata(&path).expect("sst metadata").len();
        out.push_str(&format!("file={} bytes={len}\n", path.display()));
        for entry in SstReader::open(&path)
            .expect("open SST")
            .iter()
            .expect("iter SST")
        {
            let kind = if entry.value == tombstone_value() {
                "tombstone"
            } else {
                "live"
            };
            out.push_str(&format!(
                "  key_hex={} kind={} value_len={}\n",
                hex(&entry.key),
                kind,
                entry.value.len()
            ));
        }
    }
    out
}

fn sorted_ssts(dir: &Path) -> Vec<PathBuf> {
    let mut files = fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .map(|entry| entry.path())
                .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("sst"))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    files.sort();
    files
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

fn command_text(command: &str, args: &[&str]) -> String {
    let output = Command::new(command)
        .args(args)
        .output()
        .expect("run command");
    assert!(output.status.success(), "{command} failed");
    String::from_utf8(output.stdout).expect("stdout utf8")
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().expect("vault id")
}
