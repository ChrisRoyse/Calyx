use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

use super::ProbeMatrixArgs;
use crate::error::{CliError, CliResult};

const ARTIFACT_KIND: &str = "calyx.probe_matrix.progress.v1";

pub(super) struct ProbeMatrixProgressWriter {
    path: PathBuf,
    run_dir: PathBuf,
    vault: String,
    vault_dir: PathBuf,
    args: ProbeMatrixArgs,
    events: Vec<Value>,
}

impl ProbeMatrixProgressWriter {
    pub(super) fn create(vault_dir: &Path, vault: &str, args: &ProbeMatrixArgs) -> CliResult<Self> {
        let run_dir = vault_dir
            .join("idx")
            .join("probe_matrix")
            .join("runs")
            .join(format!("{}-{}", unix_ms()?, std::process::id()));
        fs::create_dir_all(&run_dir)?;
        let mut writer = Self {
            path: run_dir.join("progress.json"),
            run_dir,
            vault: vault.to_string(),
            vault_dir: vault_dir.to_path_buf(),
            args: args.clone(),
            events: Vec::new(),
        };
        writer.write("running", "progress_artifact_created", json!({}))?;
        eprintln!("PROBE_MATRIX_PROGRESS={}", writer.path.display());
        Ok(writer)
    }

    pub(super) fn path(&self) -> &Path {
        &self.path
    }

    pub(super) fn run_dir(&self) -> &Path {
        &self.run_dir
    }

    pub(super) fn write(&mut self, status: &str, phase: &str, details: Value) -> CliResult {
        let event = json!({
            "status": status,
            "phase": phase,
            "unix_ms": unix_ms()?,
            "details": details,
        });
        self.events.push(event);
        let artifact = json!({
            "artifact_kind": ARTIFACT_KIND,
            "schema_version": 1,
            "status": status,
            "phase": phase,
            "updated_unix_ms": unix_ms()?,
            "vault": self.vault,
            "vault_dir": self.vault_dir.display().to_string(),
            "args": {
                "frontier": self.args.frontier,
                "slots": self.args.slots,
                "weighted_profiles": self.args.weighted_profiles,
                "phrasings": self.args.phrasings,
                "lengths": self.args.lengths,
                "top_k": self.args.top_k,
                "guard": format!("{:?}", self.args.guard),
                "out": self.args.out.as_ref().map(|path| path.display().to_string()),
                "max_variants": self.args.max_variants,
                "time_budget_ms": self.args.time_budget_ms,
                "resident_addr": self.args.resident_addr.map(|addr| addr.to_string()),
            },
            "events": self.events,
        });
        crate::durable_write::write_json_value_atomic(
            &self.path,
            &artifact,
            "probe matrix progress artifact",
        )
    }
}

fn unix_ms() -> CliResult<u128> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| CliError::io(format!("system clock before unix epoch: {error}")))?
        .as_millis())
}
