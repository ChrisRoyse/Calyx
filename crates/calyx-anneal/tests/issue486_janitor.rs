use calyx_anneal::{DatasetManifest, Janitor, JanitorConfig};
use calyx_aster::pressure::{DiskPressureGuard, DiskSample, DiskSpaceProbe};
use calyx_core::{FixedClock, Result};
use filetime::{FileTime, set_file_mtime};
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

fn config() -> JanitorConfig {
    JanitorConfig {
        log_max_bytes: 1_000_000,
        log_ttl: Duration::from_secs(60),
        build_artifact_keep_releases: 2,
        temp_ttl: Duration::from_secs(30),
        dataset_prune_by_manifest: true,
        log_rotation_age: Duration::from_secs(10),
        max_bytes_per_tick: 100 * 1024 * 1024,
    }
}

fn janitor(root: &Path) -> Janitor {
    Janitor::with_home(config(), Arc::new(FixedClock::new(NOW)), root)
        .with_current_exe(root.join("target").join("release-4").join("bin"))
}

fn root(name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "calyx-issue486-{name}-{}-{}",
        std::process::id(),
        NOW
    ));
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(&path).unwrap();
    path
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

#[test]
fn prune_logs_deletes_exact_ttl_bytes_and_preserves_recent_files() {
    let root = root("logs-ttl");
    let logs = root.join("logs");
    write_file(&logs.join("old-a.log.zst"), 11, 90);
    write_file(&logs.join("old-b.log.zst"), 13, 80);
    write_file(&logs.join("old-c.log.zst"), 17, 70);
    write_file(&logs.join("recent-a.log.zst"), 19, 20);
    write_file(&logs.join("recent-b.log.zst"), 23, 10);

    let result = janitor(&root).prune_logs().unwrap();

    assert_eq!(result.log_files_deleted, 3);
    assert_eq!(result.log_bytes_freed, 41);
    assert!(!logs.join("old-a.log.zst").exists());
    assert!(logs.join("recent-a.log.zst").exists());
    assert!(root.join("ledger").join("janitor.jsonl").exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn log_rotation_writes_valid_smaller_zstd_file() {
    let root = root("log-rotate");
    let log = root.join("logs").join("service.log");
    let input = vec![b'a'; 10 * 1024 * 1024];
    fs::create_dir_all(log.parent().unwrap()).unwrap();
    fs::write(&log, &input).unwrap();
    let mtime = UNIX_EPOCH + Duration::from_millis(NOW - 20_000);
    set_file_mtime(&log, FileTime::from_system_time(mtime)).unwrap();

    let result = janitor(&root).prune_logs().unwrap();
    let zst = log.with_file_name("service.log.zst");
    let decoded = zstd::decode_all(fs::File::open(&zst).unwrap()).unwrap();

    assert_eq!(result.logs_compressed, 1);
    assert!(!log.exists());
    assert!(zst.metadata().unwrap().len() < input.len() as u64);
    assert_eq!(decoded, input);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn build_artifact_prune_keeps_two_newest_and_current_binary_dir() {
    let root = root("artifacts");
    let target = root.join("target");
    for i in 0..5 {
        let dir = target.join(format!("release-{i}"));
        write_file(&dir.join("artifact.bin"), 10 + i, 100 - i as u64);
        set_dir_age(&dir, 100 - i as u64);
    }

    let result = janitor(&root).prune_build_artifacts().unwrap();

    assert_eq!(result.artifact_dirs_deleted, 3);
    assert!(!target.join("release-0").exists());
    assert!(!target.join("release-1").exists());
    assert!(!target.join("release-2").exists());
    assert!(target.join("release-3").exists());
    assert!(target.join("release-4").exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn temp_prune_removes_old_files_and_keeps_recent_files() {
    let root = root("temp");
    let tmp = root.join("vault-a").join(".tmp");
    for i in 0..4 {
        write_file(&tmp.join(format!("old-{i}.tmp")), 7, 90);
    }
    for i in 0..2 {
        write_file(&tmp.join(format!("recent-{i}.tmp")), 9, 10);
    }

    let result = janitor(&root).prune_temp_files().unwrap();

    assert_eq!(result.temp_files_deleted, 4);
    assert_eq!(result.temp_bytes_freed, 28);
    assert_eq!(fs::read_dir(&tmp).unwrap().count(), 2);
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn temp_prune_fails_closed_on_dataset_escape_and_keeps_stray() {
    let root = root("temp-stray");
    let tmp = root.join("vault-a").join(".tmp");
    let outside = root.join("outside.tmp");
    write_file(&outside, 10, 90);
    fs::create_dir_all(&tmp).unwrap();
    std::os::unix::fs::symlink(&outside, tmp.join("escape.tmp")).unwrap();

    let error = janitor(&root).prune_temp_files().unwrap_err();

    assert_eq!(error.code, "CALYX_IO_ERROR");
    assert!(outside.exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn dataset_prune_removes_only_dirs_absent_from_manifest() {
    let root = root("datasets");
    let datasets = root.join("datasets");
    for name in ["raw-a", "raw-b", "parsed-a", "quant-a", "scratch"] {
        write_file(&datasets.join(name).join("data.bin"), 5, 100);
    }
    let manifest = DatasetManifest::new(&datasets, ["raw-a", "parsed-a", "quant-a"]);

    let result = janitor(&root).prune_datasets(&manifest).unwrap();

    assert_eq!(result.dataset_dirs_deleted, 2);
    assert!(datasets.join("raw-a").exists());
    assert!(!datasets.join("raw-b").exists());
    assert!(!datasets.join("scratch").exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn zero_log_budget_deletes_all_log_files() {
    let root = root("zero-budget");
    write_file(&root.join("logs").join("a.log.zst"), 10, 1);
    write_file(&root.join("logs").join("b.log.zst"), 20, 1);
    let mut cfg = config();
    cfg.log_max_bytes = 0;
    cfg.log_rotation_age = Duration::from_secs(9_999);
    cfg.log_ttl = Duration::from_secs(9_999);
    let janitor = Janitor::with_home(cfg, Arc::new(FixedClock::new(NOW)), &root);

    let result = janitor.prune_logs().unwrap();

    assert_eq!(result.log_files_deleted, 2);
    assert_eq!(result.log_bytes_freed, 30);
    assert_eq!(fs::read_dir(root.join("logs")).unwrap().count(), 0);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn run_tick_is_proactive_when_disk_is_below_high_water() {
    let root = root("run-tick");
    write_file(&root.join("logs").join("old.log.zst"), 15, 90);
    let datasets = root.join("datasets");
    write_file(&datasets.join("keep").join("data.bin"), 5, 90);
    write_file(&datasets.join("drop").join("data.bin"), 6, 90);
    let manifest = DatasetManifest::new(&datasets, ["keep"]);
    let guard = DiskPressureGuard::with_probe(
        &root,
        0.85,
        Arc::new(FixedClock::new(NOW)),
        Arc::new(Probe {
            sample: Ok(DiskSample {
                blocks: 100,
                blocks_available: 50,
            }),
        }),
    );

    let result = janitor(&root)
        .with_dataset_manifest(manifest)
        .run_tick(&guard)
        .unwrap();

    assert!(!result.disk_pressure_before);
    assert!(result.bytes_freed >= 21);
    assert!(!datasets.join("drop").exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn compression_output_conflict_reports_io_error_and_keeps_source() {
    let root = root("compress-fail");
    let log = root.join("logs").join("bad.log");
    write_file(&log, 64, 20);
    fs::create_dir_all(log.with_file_name("bad.log.zst")).unwrap();

    let result = janitor(&root).prune_logs().unwrap();

    assert_eq!(result.logs_compressed, 0);
    assert_eq!(result.errors.len(), 1);
    assert_eq!(result.errors[0].code, "CALYX_IO_ERROR");
    assert!(log.exists());
    fs::remove_dir_all(root).unwrap();
}
