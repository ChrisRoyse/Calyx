//! Anneal-managed operational janitor for bounded hotpool buildup.

mod fs_ops;
mod types;

pub use types::{
    CALYX_IO_ERROR, DatasetManifest, GcResult, JanitorConfig, JanitorErrorReadback, JanitorMetrics,
    JanitorReadback, MAX_JANITOR_BYTES_PER_TICK,
};

use calyx_aster::pressure::DiskPressureGuard;
use calyx_core::{Clock, Result, Ts};
use fs_ops::{
    CleanupKind, age_ms, collect_files, dir_size, duration_ms, ensure_inside_dataset, file_len,
    hash_path, immediate_dirs, io_error, is_zst, modified_ms, starts_with_canonical, temp_dirs,
    zst_path,
};
use serde::Serialize;
use std::env;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

#[derive(Clone)]
pub struct Janitor {
    config: JanitorConfig,
    clock: Arc<dyn Clock>,
    home: PathBuf,
    current_exe: Option<PathBuf>,
    dataset_manifest: Option<DatasetManifest>,
    counters: Arc<Mutex<JanitorMetrics>>,
}

impl Janitor {
    pub fn new(config: JanitorConfig, clock: Arc<dyn Clock>) -> Self {
        let home = env::var_os("CALYX_HOME").map_or_else(|| PathBuf::from("."), PathBuf::from);
        Self::with_home(config, clock, home)
    }

    pub fn with_home(
        config: JanitorConfig,
        clock: Arc<dyn Clock>,
        home: impl Into<PathBuf>,
    ) -> Self {
        Self {
            config,
            clock,
            home: home.into(),
            current_exe: env::current_exe().ok(),
            dataset_manifest: None,
            counters: Arc::new(Mutex::new(JanitorMetrics::default())),
        }
    }

    pub fn with_current_exe(mut self, current_exe: impl Into<PathBuf>) -> Self {
        self.current_exe = Some(current_exe.into());
        self
    }

    pub fn with_dataset_manifest(mut self, manifest: DatasetManifest) -> Self {
        self.dataset_manifest = Some(manifest);
        self
    }

    pub fn metrics(&self) -> JanitorMetrics {
        self.counters
            .lock()
            .expect("janitor counters poisoned")
            .clone()
    }

    pub fn readback(&self) -> JanitorReadback {
        JanitorReadback {
            home: self.home.clone(),
            ledger_path: self.ledger_path(),
            metrics: self.metrics(),
        }
    }

    pub fn prometheus_text(&self, vault: &str) -> String {
        self.metrics().prometheus_text(vault)
    }

    pub fn prune_logs(&self) -> Result<GcResult> {
        let mut result = GcResult::default();
        let logs = self.home.join("logs");
        if !logs.exists() {
            return Ok(result);
        }
        let now = self.clock.now();
        for path in collect_files(&logs)? {
            if is_zst(&path) && age_ms(&path, now)? >= duration_ms(self.config.log_ttl) {
                self.delete_file(&path, CleanupKind::Log, "log_ttl_delete", &mut result)?;
            }
        }
        for path in collect_files(&logs)? {
            if !is_zst(&path) && age_ms(&path, now)? >= duration_ms(self.config.log_rotation_age) {
                self.compress_log(&path, &mut result)?;
            }
        }
        self.enforce_log_cap(&logs, &mut result)?;
        self.record_metrics(&result);
        Ok(result)
    }

    pub fn prune_build_artifacts(&self) -> Result<GcResult> {
        let mut result = GcResult::default();
        let target = self.home.join("target");
        if !target.exists() {
            return Ok(result);
        }
        let mut dirs = immediate_dirs(&target)?;
        dirs.sort_by_key(|path| modified_ms(path).unwrap_or(0));
        dirs.reverse();
        let current = self
            .current_exe
            .as_ref()
            .and_then(|path| path.canonicalize().ok());
        for (idx, dir) in dirs.into_iter().enumerate() {
            if idx < self.config.build_artifact_keep_releases {
                continue;
            }
            if current
                .as_ref()
                .is_some_and(|exe| starts_with_canonical(exe, &dir))
            {
                continue;
            }
            let bytes = dir_size(&dir)?;
            fs::remove_dir_all(&dir)
                .map_err(|error| io_error(format!("remove {}: {error}", dir.display())))?;
            result.bytes_freed = result.bytes_freed.saturating_add(bytes);
            result.artifact_bytes_freed = result.artifact_bytes_freed.saturating_add(bytes);
            result.artifact_dirs_deleted += 1;
            self.ledger_event("artifact_pruned", &dir, bytes)?;
            result.ledger_events += 1;
            result.rate_limited |= result.bytes_freed >= self.config.max_bytes_per_tick;
            if result.rate_limited {
                break;
            }
        }
        self.record_metrics(&result);
        Ok(result)
    }

    pub fn prune_temp_files(&self) -> Result<GcResult> {
        let mut result = GcResult::default();
        let now = self.clock.now();
        for temp_dir in temp_dirs(&self.home)? {
            let Some(dataset_root) = temp_dir.parent() else {
                continue;
            };
            for path in collect_files(&temp_dir)? {
                ensure_inside_dataset(dataset_root, &path)?;
                if age_ms(&path, now)? >= duration_ms(self.config.temp_ttl) {
                    self.delete_file(&path, CleanupKind::Temp, "temp_ttl_delete", &mut result)?;
                }
            }
        }
        self.record_metrics(&result);
        Ok(result)
    }

    pub fn prune_datasets(&self, manifest: &DatasetManifest) -> Result<GcResult> {
        let mut result = GcResult::default();
        if !self.config.dataset_prune_by_manifest || !manifest.datasets_dir.exists() {
            return Ok(result);
        }
        for dir in immediate_dirs(&manifest.datasets_dir)? {
            let name = dir
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_default();
            if manifest.keep.contains(name) || dir.join(".calyx-active").exists() {
                continue;
            }
            let bytes = dir_size(&dir)?;
            fs::remove_dir_all(&dir)
                .map_err(|error| io_error(format!("remove {}: {error}", dir.display())))?;
            result.bytes_freed = result.bytes_freed.saturating_add(bytes);
            result.dataset_bytes_freed = result.dataset_bytes_freed.saturating_add(bytes);
            result.dataset_dirs_deleted += 1;
            self.ledger_event("dataset_pruned", &dir, bytes)?;
            result.ledger_events += 1;
        }
        self.record_metrics(&result);
        Ok(result)
    }

    pub fn run_tick(&self, disk_pressure: &DiskPressureGuard) -> Result<GcResult> {
        let mut result = GcResult {
            disk_pressure_before: disk_pressure.check().is_err(),
            ..GcResult::default()
        };
        if result.disk_pressure_before {
            disk_pressure.request_spill();
        }
        result.merge(self.prune_temp_files()?);
        result.merge(self.prune_logs()?);
        result.merge(self.prune_build_artifacts()?);
        if let Some(manifest) = &self.dataset_manifest {
            result.merge(self.prune_datasets(manifest)?);
        }
        result.disk_pressure_after = disk_pressure.check().is_err();
        if result.disk_pressure_after {
            disk_pressure.request_spill();
        }
        Ok(result)
    }

    fn compress_log(&self, path: &Path, result: &mut GcResult) -> Result<()> {
        let before = file_len(path)?;
        let output = zst_path(path)?;
        if output.exists() {
            result.record_error(
                hash_path(path),
                io_error(format!("{} already exists", output.display())),
            );
            return Ok(());
        }
        let input = fs::read(path)
            .map_err(|error| io_error(format!("read {}: {error}", path.display())))?;
        let encoded = match zstd::encode_all(input.as_slice(), 0) {
            Ok(encoded) => encoded,
            Err(error) => {
                result.record_error(
                    hash_path(path),
                    io_error(format!("compress {}: {error}", path.display())),
                );
                return Ok(());
            }
        };
        if let Err(error) = fs::write(&output, &encoded) {
            result.record_error(
                hash_path(&output),
                io_error(format!("write {}: {error}", output.display())),
            );
            return Ok(());
        }
        fs::remove_file(path)
            .map_err(|error| io_error(format!("remove {}: {error}", path.display())))?;
        let after = file_len(&output).unwrap_or(encoded.len() as u64);
        let freed = before.saturating_sub(after);
        result.bytes_freed = result.bytes_freed.saturating_add(freed);
        result.log_bytes_freed = result.log_bytes_freed.saturating_add(freed);
        result.logs_compressed += 1;
        self.ledger_event("log_compressed", path, freed)?;
        result.ledger_events += 1;
        Ok(())
    }

    fn enforce_log_cap(&self, logs: &Path, result: &mut GcResult) -> Result<()> {
        let mut files = Vec::new();
        for path in collect_files(logs)? {
            files.push((modified_ms(&path)?, file_len(&path)?, path));
        }
        let mut total = files.iter().map(|(_, len, _)| *len).sum::<u64>();
        files.sort_by_key(|(modified, _, _)| *modified);
        for (_, len, path) in files {
            if total <= self.config.log_max_bytes {
                break;
            }
            self.delete_file(&path, CleanupKind::Log, "log_cap_delete", result)?;
            total = total.saturating_sub(len);
        }
        Ok(())
    }

    fn delete_file(
        &self,
        path: &Path,
        kind: CleanupKind,
        action: &'static str,
        result: &mut GcResult,
    ) -> Result<()> {
        let bytes = file_len(path)?;
        fs::remove_file(path)
            .map_err(|error| io_error(format!("remove {}: {error}", path.display())))?;
        result.bytes_freed = result.bytes_freed.saturating_add(bytes);
        match kind {
            CleanupKind::Log => {
                result.log_bytes_freed = result.log_bytes_freed.saturating_add(bytes);
                result.log_files_deleted += 1;
            }
            CleanupKind::Temp => {
                result.temp_bytes_freed = result.temp_bytes_freed.saturating_add(bytes);
                result.temp_files_deleted += 1;
            }
        }
        self.ledger_event(action, path, bytes)?;
        result.ledger_events += 1;
        Ok(())
    }

    fn ledger_event(&self, action: &str, path: &Path, bytes: u64) -> Result<()> {
        #[derive(Serialize)]
        struct Event<'a> {
            ts: Ts,
            action: &'a str,
            path_hash: String,
            bytes: u64,
        }

        let ledger = self.ledger_path();
        if let Some(parent) = ledger.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| io_error(format!("create {}: {error}", parent.display())))?;
        }
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&ledger)
            .map_err(|error| io_error(format!("open {}: {error}", ledger.display())))?;
        let event = Event {
            ts: self.clock.now(),
            action,
            path_hash: hash_path(path),
            bytes,
        };
        let line = serde_json::to_vec(&event)
            .map_err(|error| io_error(format!("encode janitor ledger event: {error}")))?;
        file.write_all(&line)
            .and_then(|_| file.write_all(b"\n"))
            .map_err(|error| io_error(format!("append {}: {error}", ledger.display())))
    }

    fn ledger_path(&self) -> PathBuf {
        self.home.join("ledger").join("janitor.jsonl")
    }

    fn record_metrics(&self, result: &GcResult) {
        self.counters
            .lock()
            .expect("janitor counters poisoned")
            .record(result);
    }
}
