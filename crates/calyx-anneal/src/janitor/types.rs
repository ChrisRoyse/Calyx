use calyx_core::{CalyxError, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const CALYX_IO_ERROR: &str = "CALYX_IO_ERROR";
pub const MAX_JANITOR_BYTES_PER_TICK: u64 = 100 * 1024 * 1024;

const DEFAULT_LOG_ROTATION_AGE: Duration = Duration::from_secs(60 * 60);
const DEFAULT_LOG_TTL: Duration = Duration::from_secs(7 * 24 * 60 * 60);
const DEFAULT_TEMP_TTL: Duration = Duration::from_secs(24 * 60 * 60);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JanitorConfig {
    pub log_max_bytes: u64,
    pub log_ttl: Duration,
    pub build_artifact_keep_releases: usize,
    pub temp_ttl: Duration,
    pub dataset_prune_by_manifest: bool,
    pub log_rotation_age: Duration,
    pub max_bytes_per_tick: u64,
}

impl Default for JanitorConfig {
    fn default() -> Self {
        Self {
            log_max_bytes: 256 * 1024 * 1024,
            log_ttl: DEFAULT_LOG_TTL,
            build_artifact_keep_releases: 2,
            temp_ttl: DEFAULT_TEMP_TTL,
            dataset_prune_by_manifest: false,
            log_rotation_age: DEFAULT_LOG_ROTATION_AGE,
            max_bytes_per_tick: MAX_JANITOR_BYTES_PER_TICK,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GcResult {
    pub bytes_freed: u64,
    pub log_bytes_freed: u64,
    pub artifact_bytes_freed: u64,
    pub temp_bytes_freed: u64,
    pub dataset_bytes_freed: u64,
    pub logs_compressed: usize,
    pub log_files_deleted: usize,
    pub artifact_dirs_deleted: usize,
    pub temp_files_deleted: usize,
    pub dataset_dirs_deleted: usize,
    pub ledger_events: usize,
    pub rate_limited: bool,
    pub disk_pressure_before: bool,
    pub disk_pressure_after: bool,
    pub errors: Vec<JanitorErrorReadback>,
}

impl GcResult {
    pub(super) fn merge(&mut self, other: Self) {
        self.bytes_freed = self.bytes_freed.saturating_add(other.bytes_freed);
        self.log_bytes_freed = self.log_bytes_freed.saturating_add(other.log_bytes_freed);
        self.artifact_bytes_freed = self
            .artifact_bytes_freed
            .saturating_add(other.artifact_bytes_freed);
        self.temp_bytes_freed = self.temp_bytes_freed.saturating_add(other.temp_bytes_freed);
        self.dataset_bytes_freed = self
            .dataset_bytes_freed
            .saturating_add(other.dataset_bytes_freed);
        self.logs_compressed += other.logs_compressed;
        self.log_files_deleted += other.log_files_deleted;
        self.artifact_dirs_deleted += other.artifact_dirs_deleted;
        self.temp_files_deleted += other.temp_files_deleted;
        self.dataset_dirs_deleted += other.dataset_dirs_deleted;
        self.ledger_events += other.ledger_events;
        self.rate_limited |= other.rate_limited;
        self.errors.extend(other.errors);
    }

    pub(super) fn record_error(&mut self, path_hash: String, error: CalyxError) {
        self.errors.push(JanitorErrorReadback {
            code: error.code.to_string(),
            message: error.message,
            path_hash,
        });
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct JanitorErrorReadback {
    pub code: String,
    pub message: String,
    pub path_hash: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct JanitorMetrics {
    pub bytes_freed_total: u64,
    pub log_bytes: u64,
    pub artifact_bytes: u64,
    pub temp_bytes: u64,
    pub dataset_bytes: u64,
}

impl JanitorMetrics {
    pub fn prometheus_text(&self, vault: &str) -> String {
        format!(
            "calyx_janitor_bytes_freed_total{{vault=\"{vault}\"}} {}\n\
             calyx_janitor_log_bytes{{vault=\"{vault}\"}} {}\n\
             calyx_janitor_artifact_bytes{{vault=\"{vault}\"}} {}\n\
             calyx_janitor_temp_bytes{{vault=\"{vault}\"}} {}\n\
             calyx_janitor_dataset_bytes{{vault=\"{vault}\"}} {}\n",
            self.bytes_freed_total,
            self.log_bytes,
            self.artifact_bytes,
            self.temp_bytes,
            self.dataset_bytes
        )
    }

    pub(super) fn record(&mut self, result: &GcResult) {
        self.bytes_freed_total = self.bytes_freed_total.saturating_add(result.bytes_freed);
        self.log_bytes = self.log_bytes.saturating_add(result.log_bytes_freed);
        self.artifact_bytes = self
            .artifact_bytes
            .saturating_add(result.artifact_bytes_freed);
        self.temp_bytes = self.temp_bytes.saturating_add(result.temp_bytes_freed);
        self.dataset_bytes = self
            .dataset_bytes
            .saturating_add(result.dataset_bytes_freed);
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct JanitorReadback {
    pub home: PathBuf,
    pub ledger_path: PathBuf,
    pub metrics: JanitorMetrics,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DatasetManifest {
    pub datasets_dir: PathBuf,
    pub keep: BTreeSet<String>,
}

impl DatasetManifest {
    pub fn new<I, S>(datasets_dir: impl Into<PathBuf>, keep: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            datasets_dir: datasets_dir.into(),
            keep: keep.into_iter().map(Into::into).collect(),
        }
    }

    pub fn from_json_file(
        path: impl AsRef<Path>,
        datasets_dir: impl Into<PathBuf>,
    ) -> Result<Self> {
        #[derive(Deserialize)]
        struct ManifestFile {
            datasets: Option<Vec<String>>,
            keep: Option<Vec<String>>,
        }

        let path = path.as_ref();
        let bytes = fs::read(path).map_err(|error| {
            CalyxError::dataset_manifest_invalid(format!("read {}: {error}", path.display()))
        })?;
        let manifest: ManifestFile = serde_json::from_slice(&bytes).map_err(|error| {
            CalyxError::dataset_manifest_invalid(format!("decode {}: {error}", path.display()))
        })?;
        let keep = manifest.datasets.or(manifest.keep).ok_or_else(|| {
            CalyxError::dataset_manifest_invalid("manifest must contain datasets or keep")
        })?;
        for name in &keep {
            if name.contains('/') || name.contains('\\') || name.is_empty() {
                return Err(CalyxError::dataset_manifest_invalid(format!(
                    "invalid dataset name {name:?}"
                )));
            }
        }
        Ok(Self::new(datasets_dir, keep))
    }
}
