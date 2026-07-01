use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

use super::WeaveLoomArgs;
use crate::error::{CliError, CliResult};

const ARTIFACT_KIND: &str = "calyx.weave_loom.progress.v1";

pub(super) struct WeaveLoomProgressWriter {
    path: PathBuf,
    vault: String,
    vault_dir: PathBuf,
    args: WeaveLoomArgs,
}

impl WeaveLoomProgressWriter {
    pub(super) fn create(vault_dir: &Path, vault: &str, args: &WeaveLoomArgs) -> CliResult<Self> {
        let run_dir = vault_dir
            .join("idx")
            .join("weave_loom")
            .join("runs")
            .join(format!("{}-{}", unix_ms()?, std::process::id()));
        fs::create_dir_all(&run_dir)?;
        let writer = Self {
            path: run_dir.join("progress.json"),
            vault: vault.to_string(),
            vault_dir: vault_dir.to_path_buf(),
            args: args.clone(),
        };
        writer.write("running", "progress_artifact_created", json!({}))?;
        eprintln!("WEAVE_LOOM_PROGRESS={}", writer.path.display());
        Ok(writer)
    }

    pub(super) fn path(&self) -> &Path {
        &self.path
    }

    pub(super) fn write(&self, status: &str, phase: &str, details: Value) -> CliResult {
        let artifact = json!({
            "artifact_kind": ARTIFACT_KIND,
            "schema_version": 1,
            "status": status,
            "phase": phase,
            "updated_unix_ms": unix_ms()?,
            "vault": self.vault,
            "vault_dir": self.vault_dir.display().to_string(),
            "args": {
                "content_slot": self.args.content_slot,
                "knn": self.args.knn,
                "edge_cos_threshold": self.args.edge_cos_threshold,
                "max_groundedness_distance": self.args.max_groundedness_distance,
                "batch": self.args.batch,
                "limit": self.args.limit,
                "time_budget_ms": self.args.time_budget_ms,
            },
            "details": details,
        });
        write_json_atomic(&self.path, &artifact)
    }
}

fn write_json_atomic(path: &Path, value: &Value) -> CliResult {
    let parent = path
        .parent()
        .ok_or_else(|| CliError::io(format!("progress path {} has no parent", path.display())))?;
    fs::create_dir_all(parent)?;
    let tmp = path.with_extension("json.tmp");
    let mut file = File::create(&tmp).map_err(|error| {
        CliError::io(format!(
            "create temporary progress artifact {} failed: {error}",
            tmp.display()
        ))
    })?;
    file.write_all(&serde_json::to_vec_pretty(value)?)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    drop(file);
    fs::rename(&tmp, path).map_err(|error| {
        CliError::io(format!(
            "publish progress artifact {} -> {} failed: {error}",
            tmp.display(),
            path.display()
        ))
    })?;
    sync_parent_dir(parent)?;
    Ok(())
}

#[cfg(unix)]
fn sync_parent_dir(parent: &Path) -> CliResult {
    let dir = File::open(parent).map_err(|error| {
        CliError::io(format!(
            "open progress artifact parent directory {} for sync failed: {error}",
            parent.display()
        ))
    })?;
    dir.sync_all().map_err(|error| {
        CliError::io(format!(
            "sync progress artifact parent directory {} failed: {error}",
            parent.display()
        ))
    })
}

#[cfg(windows)]
fn sync_parent_dir(parent: &Path) -> CliResult {
    use std::fs::OpenOptions;
    use std::os::windows::fs::OpenOptionsExt;

    const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;

    let dir = OpenOptions::new()
        .read(true)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
        .open(parent)
        .map_err(|error| {
            CliError::io(format!(
                "open progress artifact parent directory {} for sync failed: {error}",
                parent.display()
            ))
        })?;
    dir.sync_all().map_err(|error| {
        CliError::io(format!(
            "sync progress artifact parent directory {} failed: {error}",
            parent.display()
        ))
    })
}

#[cfg(not(any(unix, windows)))]
fn sync_parent_dir(parent: &Path) -> CliResult {
    Err(CliError::io(format!(
        "sync progress artifact parent directory {} is unsupported on this platform",
        parent.display()
    )))
}

fn unix_ms() -> CliResult<u128> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| CliError::io(format!("system clock before unix epoch: {error}")))?
        .as_millis())
}
