use std::fs;
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
        crate::durable_write::write_json_value_atomic(
            &self.path,
            &artifact,
            "weave loom progress artifact",
        )
    }
}

fn unix_ms() -> CliResult<u128> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| CliError::io(format!("system clock before unix epoch: {error}")))?
        .as_millis())
}
