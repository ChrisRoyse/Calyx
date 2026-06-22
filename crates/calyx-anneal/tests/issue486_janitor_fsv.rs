use calyx_anneal::{DatasetManifest, Janitor, JanitorConfig};
use calyx_aster::pressure::{DiskPressureGuard, DiskSample, DiskSpaceProbe};
use calyx_core::{FixedClock, Result};
use filetime::{FileTime, set_file_mtime};
use serde::Serialize;
use serde_json::json;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, UNIX_EPOCH};

const NOW: u64 = 1_800_000_000_000;

#[derive(Debug)]
struct Probe {
    sample: Result<DiskSample>,
}

impl DiskSpaceProbe for Probe {
    fn sample(&self, _path: &Path) -> Result<DiskSample> {
        self.sample.clone()
    }
}

#[derive(Serialize)]
struct InventoryEntry {
    path: String,
    kind: &'static str,
    bytes: u64,
}

fn config(log_max_bytes: u64) -> JanitorConfig {
    JanitorConfig {
        log_max_bytes,
        log_ttl: Duration::from_secs(600),
        build_artifact_keep_releases: 2,
        temp_ttl: Duration::from_secs(30),
        dataset_prune_by_manifest: true,
        log_rotation_age: Duration::from_secs(10),
        max_bytes_per_tick: 100 * 1024 * 1024,
    }
}

fn janitor(root: &Path, log_max_bytes: u64) -> Janitor {
    Janitor::with_home(config(log_max_bytes), Arc::new(FixedClock::new(NOW)), root)
        .with_current_exe(root.join("target").join("release-4").join("bin"))
}

fn fsv_root() -> PathBuf {
    std::env::var_os("CALYX_FSV_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(format!(
                "/var/lib/calyx/data/fsv-issue486-{}",
                std::process::id()
            ))
        })
}

fn write_file(path: &Path, len: usize, age_secs: u64) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    let mut file = fs::File::create(path).unwrap();
    file.write_all(&vec![b'x'; len]).unwrap();
    let mtime = UNIX_EPOCH + Duration::from_millis(NOW - age_secs * 1_000);
    set_file_mtime(path, FileTime::from_system_time(mtime)).unwrap();
}

fn set_dir_age(path: &Path, age_secs: u64) {
    fs::create_dir_all(path).unwrap();
    let mtime = UNIX_EPOCH + Duration::from_millis(NOW - age_secs * 1_000);
    set_file_mtime(path, FileTime::from_system_time(mtime)).unwrap();
}

fn tree_bytes(path: &Path) -> u64 {
    if !path.exists() {
        return 0;
    }
    let metadata = fs::symlink_metadata(path).unwrap();
    if metadata.is_file() {
        return metadata.len();
    }
    if metadata.is_dir() {
        return fs::read_dir(path)
            .unwrap()
            .map(|entry| tree_bytes(&entry.unwrap().path()))
            .sum();
    }
    metadata.len()
}

fn inventory(root: &Path) -> Vec<InventoryEntry> {
    let mut entries = Vec::new();
    collect_inventory(root, root, &mut entries);
    entries.sort_by(|left, right| left.path.cmp(&right.path));
    entries
}

fn collect_inventory(base: &Path, path: &Path, entries: &mut Vec<InventoryEntry>) {
    if !path.exists() {
        return;
    }
    let metadata = fs::symlink_metadata(path).unwrap();
    let kind = if metadata.is_dir() {
        "dir"
    } else if metadata.is_file() {
        "file"
    } else {
        "other"
    };
    let relative = path
        .strip_prefix(base)
        .unwrap()
        .to_string_lossy()
        .replace('\\', "/");
    if !relative.is_empty() {
        entries.push(InventoryEntry {
            path: relative,
            kind,
            bytes: metadata.len(),
        });
    }
    if metadata.is_dir() {
        for entry in fs::read_dir(path).unwrap() {
            collect_inventory(base, &entry.unwrap().path(), entries);
        }
    }
}

fn run_happy(root: &Path) -> serde_json::Value {
    let logs = root.join("logs");
    for (idx, age) in [80, 70, 10, 5].iter().enumerate() {
        write_file(&logs.join(format!("cap-{idx}.log.zst")), 40, *age);
    }
    let target = root.join("target");
    for i in 0..5 {
        let dir = target.join(format!("release-{i}"));
        write_file(&dir.join("artifact.bin"), 11 + i, 100 - i as u64);
        set_dir_age(&dir, 100 - i as u64);
    }
    let tmp = root.join("vault-a").join(".tmp");
    for i in 0..4 {
        write_file(&tmp.join(format!("old-{i}.tmp")), 7, 90);
    }
    for i in 0..2 {
        write_file(&tmp.join(format!("recent-{i}.tmp")), 9, 10);
    }
    let datasets = root.join("datasets");
    for (name, bytes) in [
        ("raw-a", 5),
        ("parsed-a", 6),
        ("quant-a", 7),
        ("raw-b", 5),
        ("scratch", 6),
    ] {
        write_file(&datasets.join(name).join("data.bin"), bytes, 90);
    }
    let before = inventory(root);
    let before_du = json!({
        "logs": tree_bytes(&logs),
        "target": tree_bytes(&target),
        "temp": tree_bytes(&root.join("vault-a")),
        "datasets": tree_bytes(&datasets),
    });
    let manifest = DatasetManifest::new(&datasets, ["raw-a", "parsed-a", "quant-a"]);
    let guard = DiskPressureGuard::with_probe(
        root,
        0.85,
        Arc::new(FixedClock::new(NOW)),
        Arc::new(Probe {
            sample: Ok(DiskSample {
                blocks: 100,
                blocks_available: 50,
            }),
        }),
    );
    let janitor = janitor(root, 80).with_dataset_manifest(manifest);
    let result = janitor.run_tick(&guard).unwrap();
    assert_eq!(result.log_files_deleted, 2);
    assert_eq!(result.artifact_dirs_deleted, 3);
    assert_eq!(result.temp_files_deleted, 4);
    assert_eq!(result.dataset_dirs_deleted, 2);
    assert_eq!(tree_bytes(&logs), 80);
    let metrics = janitor.prometheus_text("issue486-happy");
    json!({
        "trigger": "Janitor::run_tick with disk pressure below high-water",
        "expected": {
            "log_cap_before_bytes": 160,
            "log_cap_after_max_bytes": 80,
            "log_bytes_freed": 80,
            "artifact_dirs_deleted": 3,
            "artifact_bytes_freed": 36,
            "temp_files_deleted": 4,
            "temp_bytes_freed": 28,
            "dataset_dirs_deleted": 2,
            "dataset_bytes_freed": 11,
            "disk_pressure_before": false
        },
        "before_du": before_du,
        "after_du": {
            "logs": tree_bytes(&logs),
            "target": tree_bytes(&target),
            "temp": tree_bytes(&root.join("vault-a")),
            "datasets": tree_bytes(&datasets)
        },
        "before_inventory": before,
        "after_inventory": inventory(root),
        "result": result,
        "readback": janitor.readback(),
        "metrics": metrics,
    })
}

fn run_rotation(root: &Path) -> serde_json::Value {
    let log = root.join("logs").join("service.log");
    let input = vec![b'a'; 1024 * 1024];
    fs::create_dir_all(log.parent().unwrap()).unwrap();
    fs::write(&log, &input).unwrap();
    let mtime = UNIX_EPOCH + Duration::from_millis(NOW - 20_000);
    set_file_mtime(&log, FileTime::from_system_time(mtime)).unwrap();
    let before = inventory(root);
    let janitor = janitor(root, 2 * 1024 * 1024);
    let result = janitor.prune_logs().unwrap();
    let zst = log.with_file_name("service.log.zst");
    let decoded = zstd::decode_all(fs::File::open(&zst).unwrap()).unwrap();
    assert_eq!(decoded.len(), input.len());
    assert!(!log.exists());
    json!({
        "trigger": "Janitor::prune_logs rotates an old raw log",
        "expected": {
            "source_absent": true,
            "zstd_valid_decoded_bytes": input.len(),
            "compressed_smaller_than_source": true
        },
        "before_inventory": before,
        "after_inventory": inventory(root),
        "result": result,
        "zst_bytes": fs::metadata(&zst).unwrap().len(),
        "decoded_bytes": decoded.len(),
        "metrics": janitor.prometheus_text("issue486-rotation")
    })
}

fn run_zero_budget(root: &Path) -> serde_json::Value {
    write_file(&root.join("logs").join("a.log.zst"), 10, 1);
    write_file(&root.join("logs").join("b.log.zst"), 20, 1);
    let before = inventory(root);
    let mut cfg = config(0);
    cfg.log_rotation_age = Duration::from_secs(9_999);
    cfg.log_ttl = Duration::from_secs(9_999);
    let janitor = Janitor::with_home(cfg, Arc::new(FixedClock::new(NOW)), root);
    let result = janitor.prune_logs().unwrap();
    assert_eq!(result.log_bytes_freed, 30);
    assert_eq!(fs::read_dir(root.join("logs")).unwrap().count(), 0);
    json!({
        "trigger": "Janitor::prune_logs with log_max_bytes=0",
        "expected": {"all_log_files_deleted": true, "log_bytes_freed": 30},
        "before_inventory": before,
        "after_inventory": inventory(root),
        "result": result,
        "metrics": janitor.prometheus_text("issue486-zero-budget")
    })
}

fn run_compression_conflict(root: &Path) -> serde_json::Value {
    let log = root.join("logs").join("bad.log");
    write_file(&log, 64, 20);
    fs::create_dir_all(log.with_file_name("bad.log.zst")).unwrap();
    let before = inventory(root);
    let janitor = janitor(root, 1_000);
    let result = janitor.prune_logs().unwrap();
    assert_eq!(result.errors[0].code, "CALYX_IO_ERROR");
    assert!(log.exists());
    json!({
        "trigger": "Janitor::prune_logs sees conflicting .zst destination",
        "expected": {"error_code": "CALYX_IO_ERROR", "source_log_kept": true},
        "before_inventory": before,
        "after_inventory": inventory(root),
        "result": result,
        "source_exists_after": log.exists()
    })
}

#[cfg(unix)]
fn run_temp_escape(root: &Path) -> serde_json::Value {
    let tmp = root.join("vault-a").join(".tmp");
    let outside = root.join("outside.tmp");
    write_file(&outside, 10, 90);
    fs::create_dir_all(&tmp).unwrap();
    std::os::unix::fs::symlink(&outside, tmp.join("escape.tmp")).unwrap();
    let before = inventory(root);
    let err = janitor(root, 1_000).prune_temp_files().unwrap_err();
    assert_eq!(err.code, "CALYX_IO_ERROR");
    assert!(outside.exists());
    json!({
        "trigger": "Janitor::prune_temp_files follows a temp symlink outside its dataset",
        "expected": {"error_code": "CALYX_IO_ERROR", "outside_file_preserved": true},
        "before_inventory": before,
        "after_inventory": inventory(root),
        "error": {"code": err.code, "message": err.message},
        "outside_exists_after": outside.exists()
    })
}

#[cfg(not(unix))]
fn run_temp_escape(root: &Path) -> serde_json::Value {
    json!({
        "skipped": true,
        "reason": "symlink escape FSV is run in a manual verification run Linux",
        "root": root
    })
}

#[test]
#[ignore = "manual FSV trigger; inspect durable bytes separately"]
fn issue486_janitor_manual_fsv_bytes() {
    let root = fsv_root();
    assert!(
        !root.exists(),
        "choose a fresh CALYX_FSV_ROOT; refusing to overwrite {}",
        root.display()
    );
    fs::create_dir_all(&root).unwrap();
    let happy = run_happy(&root.join("happy"));
    let rotation = run_rotation(&root.join("rotation"));
    let zero_budget = run_zero_budget(&root.join("zero-budget"));
    let compression_conflict = run_compression_conflict(&root.join("compression-conflict"));
    let temp_escape = run_temp_escape(&root.join("temp-escape"));
    let summary = json!({
        "issue": 486,
        "source_of_truth": {
            "root": root,
            "happy_ledger": root.join("happy").join("ledger").join("janitor.jsonl"),
            "metrics": root.join("metrics.prom"),
            "summary": root.join("issue486-summary.json")
        },
        "happy": happy,
        "rotation": rotation,
        "zero_budget": zero_budget,
        "compression_conflict": compression_conflict,
        "temp_escape": temp_escape
    });
    fs::write(
        root.join("metrics.prom"),
        summary["happy"]["metrics"].as_str().unwrap(),
    )
    .unwrap();
    fs::write(
        root.join("issue486-summary.json"),
        serde_json::to_vec_pretty(&summary).unwrap(),
    )
    .unwrap();
    println!("ISSUE486_FSV_ROOT={}", root.display());
}
