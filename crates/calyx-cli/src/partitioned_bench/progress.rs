use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use calyx_sextant::index::{DiskAnnBuildBackend, PartitionBuildParams, PartitionDistanceMetric};
use serde::Serialize;

use crate::error::{CliError, CliResult};

const FORMAT: &str = "calyx-partitioned-build-progress-v1";
const POLL_INTERVAL: Duration = Duration::from_secs(5);

#[derive(Clone)]
pub(crate) struct BuildProgressConfig {
    pub(crate) vault: PathBuf,
    pub(crate) params: PartitionBuildParams,
    pub(crate) backend: DiskAnnBuildBackend,
    pub(crate) distance_metric: PartitionDistanceMetric,
}

pub(crate) struct BuildProgress {
    active: Option<ActiveProgress>,
}

struct ActiveProgress {
    path: PathBuf,
    config: BuildProgressConfig,
    started: Instant,
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

#[derive(Serialize)]
struct Snapshot<'a> {
    format: &'static str,
    trigger: &'static str,
    phase: String,
    exit_code: Option<u8>,
    error_code: Option<&'a str>,
    error_message: Option<&'a str>,
    elapsed_ms: u128,
    vault: String,
    geometry: GeometrySnapshot,
    counts: CountSnapshot,
}

#[derive(Serialize)]
struct GeometrySnapshot {
    n_cx: u64,
    dim: usize,
    requested_regions: usize,
    sample: usize,
    chunk: usize,
    m_max: usize,
    ef_construction: usize,
    region_build_parallelism: usize,
    final_assignment_probe: usize,
    final_assignment_cap: Option<usize>,
    build_backend: &'static str,
    distance_metric: &'static str,
}

#[derive(Serialize)]
struct CountSnapshot {
    assign_initial_files: usize,
    final_ids_files: usize,
    graph_files: usize,
    manifest_exists: bool,
}

impl BuildProgress {
    pub(crate) fn start(path: Option<&Path>, config: BuildProgressConfig) -> CliResult<Self> {
        let Some(path) = path else {
            return Ok(Self { active: None });
        };
        let active = ActiveProgress::start(path.to_path_buf(), config)?;
        Ok(Self {
            active: Some(active),
        })
    }

    pub(crate) fn complete(mut self) -> CliResult {
        if let Some(mut active) = self.active.take() {
            active.stop_thread();
            active.write_terminal("complete", 0, None)?;
        }
        Ok(())
    }

    pub(crate) fn fail(mut self, error: &CliError) -> CliResult {
        if let Some(mut active) = self.active.take() {
            active.stop_thread();
            active.write_terminal("failed", 2, Some(error))?;
        }
        Ok(())
    }
}

pub(crate) fn write_failure(
    path: Option<&Path>,
    config: BuildProgressConfig,
    error: &CliError,
) -> CliResult {
    let Some(path) = path else {
        return Ok(());
    };
    write_snapshot(
        path,
        &config,
        Instant::now(),
        Some("failed"),
        Some(2),
        Some(error),
    )
}

impl ActiveProgress {
    fn start(path: PathBuf, config: BuildProgressConfig) -> CliResult<Self> {
        let started = Instant::now();
        write_snapshot(&path, &config, started, None, None, None)?;
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
        let thread_path = path.clone();
        let thread_config = config.clone();
        let handle = thread::spawn(move || {
            while !thread_stop.load(Ordering::Relaxed) {
                let _ = write_snapshot(&thread_path, &thread_config, started, None, None, None);
                thread::park_timeout(POLL_INTERVAL);
            }
        });
        Ok(Self {
            path,
            config,
            started,
            stop,
            handle: Some(handle),
        })
    }

    fn stop_thread(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            handle.thread().unpark();
            let _ = handle.join();
        }
    }

    fn write_terminal(&self, phase: &str, exit_code: u8, error: Option<&CliError>) -> CliResult {
        write_snapshot(
            &self.path,
            &self.config,
            self.started,
            Some(phase),
            Some(exit_code),
            error,
        )
    }
}

fn write_snapshot(
    path: &Path,
    config: &BuildProgressConfig,
    started: Instant,
    phase_override: Option<&str>,
    exit_code: Option<u8>,
    error: Option<&CliError>,
) -> CliResult {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)
            .map_err(|e| CliError::io(format!("create progress dir: {e}")))?;
    }
    let counts = counts(&config.vault);
    let phase = phase_override
        .map(str::to_string)
        .unwrap_or_else(|| infer_phase(&counts, config.params.n_regions));
    let snapshot = Snapshot {
        format: FORMAT,
        trigger: "calyx build-partitioned-vault",
        phase,
        exit_code,
        error_code: error.map(CliError::code),
        error_message: error.map(CliError::message),
        elapsed_ms: started.elapsed().as_millis(),
        vault: config.vault.to_string_lossy().into_owned(),
        geometry: geometry(config),
        counts,
    };
    let bytes = serde_json::to_vec_pretty(&snapshot).map_err(CliError::from)?;
    write_atomic(path, &bytes)
}

fn geometry(config: &BuildProgressConfig) -> GeometrySnapshot {
    let params = config.params;
    GeometrySnapshot {
        n_cx: params.n_cx,
        dim: params.dim,
        requested_regions: params.n_regions,
        sample: params.sample,
        chunk: params.chunk,
        m_max: params.m_max,
        ef_construction: params.ef_construction,
        region_build_parallelism: params.region_build_parallelism,
        final_assignment_probe: params.final_assignment_probe,
        final_assignment_cap: params.final_assignment_cap,
        build_backend: config.backend.as_str(),
        distance_metric: config.distance_metric.as_str(),
    }
}

fn infer_phase(counts: &CountSnapshot, requested_regions: usize) -> String {
    if counts.manifest_exists {
        return "manifest_written".to_string();
    }
    if counts.graph_files > 0 {
        return "graph_build".to_string();
    }
    if counts.final_ids_files > 0 {
        return "final_assignment".to_string();
    }
    if counts.assign_initial_files >= requested_regions.max(1) {
        return "balancing_or_final_assignment_pending".to_string();
    }
    if counts.assign_initial_files > 0 {
        return "initial_assignment".to_string();
    }
    "sampling_or_initializing".to_string()
}

fn counts(vault: &Path) -> CountSnapshot {
    CountSnapshot {
        assign_initial_files: count_ids(&vault.join("idx/assign-initial")),
        final_ids_files: count_ids(&vault.join("idx")),
        graph_files: count_graphs(&vault.join("idx")),
        manifest_exists: vault.join("partitioned-manifest.json").is_file(),
    }
}

fn count_ids(dir: &Path) -> usize {
    let Ok(entries) = fs::read_dir(dir) else {
        return 0;
    };
    entries
        .flatten()
        .filter(|entry| {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            name.starts_with("region_") && name.ends_with(".ids")
        })
        .count()
}

fn count_graphs(idx_dir: &Path) -> usize {
    let mut count = usize::from(idx_dir.join("slot_00.ann/graph.cda").is_file());
    let Ok(entries) = fs::read_dir(idx_dir) else {
        return count;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with("region_") && name.ends_with(".ann") {
            count += usize::from(entry.path().join("graph.cda").is_file());
        }
    }
    count
}

fn write_atomic(path: &Path, bytes: &[u8]) -> CliResult {
    let tmp = path.with_extension(format!("tmp-{}", std::process::id()));
    fs::write(&tmp, bytes).map_err(|e| CliError::io(format!("write progress: {e}")))?;
    #[cfg(windows)]
    if path.exists() {
        fs::remove_file(path).map_err(|e| CliError::io(format!("replace progress: {e}")))?;
    }
    fs::rename(&tmp, path).map_err(|e| CliError::io(format!("rename progress: {e}")))?;
    Ok(())
}
