#![cfg(target_os = "linux")]

//! PH58 T02 manual FSV driver for MVCC snapshot GC.

use calyx_aster::cf::ColumnFamily;
use calyx_aster::gc::{CALYX_GC_ERROR, GcRateLimit};
use calyx_aster::resource::{ResourceStatus, VramBudgetStatus};
use calyx_aster::sst::SstReader;
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{Clock, Ts, VaultId};
use serde_json::json;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

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
#[ignore = "manual FSV writes PH58 MVCC snapshot-GC source-of-truth artifacts"]
fn ph58_mvcc_snapshot_gc_fsv() {
    let root = fsv_root_os("CALYX_FSV_ROOT", "calyx-ph58-mvcc-gc");
    reset_dir(&root);
    fs::write(
        root.join("cleanup-tag.txt"),
        b"issue482 PH58 MVCC snapshot GC synthetic FSV data\n",
    )
    .expect("write cleanup tag");

    let happy = run_happy_path(&root.join("happy"));
    let capped = run_rate_limit_edge(&root.join("rate-limit"));
    let physical_cap = run_physical_cap_edge(&root.join("physical-cap"));
    let boundary = run_boundary_edge(&root.join("boundary"));
    let invalid_env = run_invalid_env_edge();
    let summary = json!({
        "issue": 482,
        "trigger": "SnapshotGcReclaimer tick after known MVCC overwrites",
        "outcome": "versions older than the safe point are reclaimed and old SST inputs are compacted away",
        "happy": happy,
        "rate_limit_edge": capped,
        "physical_cap_edge": physical_cap,
        "boundary_edge": boundary,
        "invalid_env_edge": invalid_env
    });
    fs::write(
        root.join("ph58-mvcc-snapshot-gc-readback.json"),
        serde_json::to_vec_pretty(&summary).expect("encode summary"),
    )
    .expect("write summary");
    println!(
        "PH58 MVCC snapshot GC FSV: happy_reclaimed={} happy_debt={} capped_reclaimed={} boundary_reclaimed={}",
        summary["happy"]["actual"]["gc_result"]["versions_reclaimed"],
        summary["happy"]["actual"]["status_after"]["gc"]["compaction_debt"],
        summary["rate_limit_edge"]["actual"]["gc_result"]["versions_reclaimed"],
        summary["boundary_edge"]["actual"]["gc_result"]["versions_reclaimed"]
    );
}

fn run_happy_path(root: &Path) -> serde_json::Value {
    reset_dir(root);
    let vault_dir = root.join("vault");
    let clock = SharedClock::new(10_000);
    let vault = open_vault(&vault_dir, clock);
    let key = b"ph58-mvcc-key".to_vec();
    write_versions(&vault, &key, 128, 4096);
    let safe_point = 64;
    let reader_id = vault.pin_reader_at(safe_point, 60_000);
    vault.flush().expect("flush before GC");
    write_artifacts(root, "before", &vault, &vault_dir);
    let status_before = status(&vault, &vault_dir);
    let sst_bytes_before = sst_bytes(&vault_dir);

    let result = vault
        .snapshot_version_gc_once(GcRateLimit::new(1_000, Duration::ZERO))
        .expect("snapshot GC run");
    vault.flush().expect("flush after GC");
    write_artifacts(root, "after", &vault, &vault_dir);
    let status_after = status(&vault, &vault_dir);
    let sst_bytes_after = sst_bytes(&vault_dir);
    let visible_at_safe = vault
        .read_cf_at(safe_point, ColumnFamily::Base, &key)
        .expect("safe read")
        .expect("value at safe point");
    let latest = vault
        .read_cf_at(vault.latest_seq(), ColumnFamily::Base, &key)
        .expect("latest read")
        .expect("latest value");
    assert_eq!(result.safe_point_seq, safe_point);
    assert_eq!(result.versions_reclaimed, 63);
    assert_eq!(status_before.gc.compaction_debt, 63);
    assert_eq!(status_after.gc.compaction_debt, 0);
    assert_eq!(visible_at_safe, known_value(safe_point, 4096));
    assert_eq!(latest, known_value(128, 4096));
    assert!(sst_bytes_after < sst_bytes_before);
    assert!(status_after.gc.bytes_freed_total > 0);
    assert!(vault.release_reader(reader_id));
    json!({
        "input": {"versions": 128, "safe_point": safe_point, "value_bytes": 4096},
        "expected": {"versions_reclaimed": 63, "debt_before": 63, "debt_after": 0, "sst_bytes_decrease": true},
        "actual": {
            "gc_result": result,
            "status_before": status_before,
            "status_after": status_after,
            "sst_bytes_before": sst_bytes_before,
            "sst_bytes_after": sst_bytes_after,
            "visible_at_safe_prefix": String::from_utf8_lossy(&visible_at_safe[..16]).to_string(),
            "latest_prefix": String::from_utf8_lossy(&latest[..16]).to_string()
        }
    })
}

fn run_rate_limit_edge(root: &Path) -> serde_json::Value {
    reset_dir(root);
    let vault_dir = root.join("vault");
    let clock = SharedClock::new(20_000);
    let vault = open_vault(&vault_dir, clock);
    let key = b"ph58-rate-key".to_vec();
    write_versions(&vault, &key, 25, 512);
    let safe_point = 20;
    let reader_id = vault.pin_reader_at(safe_point, 60_000);
    vault.flush().expect("flush before capped GC");
    write_artifacts(root, "before", &vault, &vault_dir);

    let result = vault
        .snapshot_version_gc_once(GcRateLimit::new(5, Duration::ZERO))
        .expect("capped GC");
    let after = status(&vault, &vault_dir);
    write_artifacts(root, "after", &vault, &vault_dir);
    assert_eq!(result.versions_reclaimed, 5);
    assert_eq!(after.gc.compaction_debt, 14);
    assert!(result.rate_limited);
    assert!(vault.release_reader(reader_id));
    json!({
        "input": {"versions": 25, "safe_point": safe_point, "max_ops_per_run": 5},
        "expected": {"versions_reclaimed": 5, "remaining_debt": 14, "rate_limited": true},
        "actual": {"gc_result": result, "status_after": after}
    })
}

fn run_physical_cap_edge(root: &Path) -> serde_json::Value {
    reset_dir(root);
    let vault_dir = root.join("vault");
    let clock = SharedClock::new(25_000);
    let vault = open_vault(&vault_dir, clock);
    let key = b"ph58-physical-cap-key".to_vec();
    write_versions(&vault, &key, 25, 256);
    let safe_point = 20;
    let reader_id = vault.pin_reader_at(safe_point, 60_000);
    vault.flush().expect("flush before physical capped GC");
    write_artifacts(root, "before", &vault, &vault_dir);

    let result = vault
        .snapshot_version_gc_once(GcRateLimit::new(19, Duration::ZERO))
        .expect("physical capped GC");
    let after = status(&vault, &vault_dir);
    write_artifacts(root, "after", &vault, &vault_dir);
    assert_eq!(result.versions_reclaimed, 19);
    assert_eq!(result.compaction_debt, 0);
    assert_eq!(after.gc.compaction_debt, 0);
    assert!(result.rate_limited);
    assert!(vault.release_reader(reader_id));
    json!({
        "input": {"versions": 25, "safe_point": safe_point, "max_ops_per_run": 19},
        "expected": {
            "versions_reclaimed": 19,
            "remaining_memory_debt": 0,
            "physical_sst_cap_reported": true
        },
        "actual": {"gc_result": result, "status_after": after}
    })
}

fn run_boundary_edge(root: &Path) -> serde_json::Value {
    reset_dir(root);
    let vault_dir = root.join("vault");
    let clock = SharedClock::new(30_000);
    let vault = open_vault(&vault_dir, clock);
    let key = b"ph58-boundary-key".to_vec();
    write_versions(&vault, &key, 1, 128);
    let reader_id = vault.pin_reader_at(vault.latest_seq(), 60_000);
    write_artifacts(root, "before", &vault, &vault_dir);

    let result = vault
        .snapshot_version_gc_once(GcRateLimit::new(100, Duration::ZERO))
        .expect("boundary GC");
    let after = status(&vault, &vault_dir);
    write_artifacts(root, "after", &vault, &vault_dir);
    assert_eq!(result.versions_reclaimed, 0);
    assert_eq!(after.gc.compaction_debt, 0);
    assert!(vault.release_reader(reader_id));
    json!({
        "input": {"versions": 1, "safe_point": 1},
        "expected": {"versions_reclaimed": 0, "compaction_debt": 0},
        "actual": {"gc_result": result, "status_after": after}
    })
}

fn run_invalid_env_edge() -> serde_json::Value {
    unsafe {
        std::env::set_var("CALYX_GC_MAX_OPS_PER_RUN", "not-a-number");
    }
    let error = GcRateLimit::from_env().expect_err("invalid env fails closed");
    unsafe {
        std::env::remove_var("CALYX_GC_MAX_OPS_PER_RUN");
    }
    assert_eq!(error.code, CALYX_GC_ERROR);
    json!({
        "input": {"CALYX_GC_MAX_OPS_PER_RUN": "not-a-number"},
        "expected": {"error_code": CALYX_GC_ERROR},
        "actual": {"error_code": error.code, "message": error.message}
    })
}

fn open_vault(vault_dir: &Path, clock: SharedClock) -> AsterVault<SharedClock> {
    fs::create_dir_all(vault_dir).expect("create vault dir");
    AsterVault::new_durable_with_clock(
        vault_dir,
        vault_id(),
        b"issue482-ph58-mvcc-snapshot-gc".to_vec(),
        VaultOptions::default(),
        clock,
    )
    .expect("open durable vault")
}

fn write_versions(vault: &AsterVault<SharedClock>, key: &[u8], versions: u64, value_bytes: usize) {
    for seq in 1..=versions {
        let committed = vault
            .write_cf(
                ColumnFamily::Base,
                key.to_vec(),
                known_value(seq, value_bytes),
            )
            .expect("version write");
        assert_eq!(committed, seq);
    }
}

fn known_value(seq: u64, value_bytes: usize) -> Vec<u8> {
    let mut value = format!("ph58-v{seq:04}:").into_bytes();
    value.resize(value_bytes, b'x');
    value
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

fn write_artifacts(root: &Path, phase: &str, vault: &AsterVault<SharedClock>, vault_dir: &Path) {
    let status = status(vault, vault_dir);
    fs::write(
        root.join(format!("status-{phase}.json")),
        serde_json::to_vec_pretty(&status).expect("status json"),
    )
    .expect("write status");
    fs::write(
        root.join(format!("metrics-{phase}.prom")),
        status.to_metrics_text("issue482"),
    )
    .expect("write metrics");
    fs::write(root.join(format!("du-{phase}.txt")), du_text(vault_dir)).expect("write du");
    fs::write(root.join(format!("df-{phase}.txt")), df_text(vault_dir)).expect("write df");
    fs::write(
        root.join(format!("base-cf-{phase}.txt")),
        dump_base_cf(vault_dir),
    )
    .expect("write base CF dump");
    fs::write(root.join(format!("wal-{phase}.txt")), list_wal(vault_dir)).expect("write WAL list");
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
            out.push_str(&format!(
                "  key_hex={} value_len={} value_prefix_hex={}\n",
                hex(&entry.key),
                entry.value.len(),
                hex(&entry.value[..entry.value.len().min(24)])
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

fn list_wal(vault_dir: &Path) -> String {
    let mut files = sorted_files(&vault_dir.join("wal"), "wal");
    let mut out = String::new();
    for path in files.drain(..) {
        let len = fs::metadata(&path).expect("wal metadata").len();
        out.push_str(&format!("file={} bytes={len}\n", path.display()));
    }
    out
}

fn sorted_files(dir: &Path, extension: &str) -> Vec<PathBuf> {
    let mut files = fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .map(|entry| entry.path())
                .filter(|path| path.extension().and_then(|value| value.to_str()) == Some(extension))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    files.sort();
    files
}

fn sst_bytes(vault_dir: &Path) -> u64 {
    sorted_ssts(&vault_dir.join("cf/base"))
        .into_iter()
        .map(|path| fs::metadata(path).expect("sst metadata").len())
        .sum()
}

fn du_text(path: &Path) -> String {
    command_text("du", &["-sb", path.to_str().expect("path utf8")])
}

fn df_text(path: &Path) -> String {
    command_text("df", &["-B1", path.to_str().expect("path utf8")])
}

fn command_text(command: &str, args: &[&str]) -> String {
    let output = Command::new(command)
        .args(args)
        .output()
        .expect("run command");
    assert!(
        output.status.success(),
        "{command} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("stdout utf8")
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().expect("vault id")
}
