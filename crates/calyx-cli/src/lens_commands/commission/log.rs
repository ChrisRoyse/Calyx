use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

use calyx_core::CalyxError;
use serde::Serialize;
use serde_json::json;

use crate::error::{CliError, CliResult};

pub(super) struct ConversionLog {
    pub(super) path: PathBuf,
}

impl ConversionLog {
    pub(super) fn create(path: PathBuf) -> CliResult<Self> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        File::create(&path)?.sync_all()?;
        Ok(Self { path })
    }

    pub(super) fn event(&mut self, value: serde_json::Value) -> CliResult {
        let mut file = OpenOptions::new().append(true).open(&self.path)?;
        let bytes = serde_json::to_vec(&value)?;
        file.write_all(&bytes)?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        Ok(())
    }
}

pub(super) fn run_command(log: &mut ConversionLog, program: &str, args: &[&str]) -> CliResult {
    log.event(json!({"event": "command_start", "program": program, "args": args}))?;
    let output = Command::new(program)
        .args(args)
        .output()
        .map_err(|err| CalyxError::lens_unreachable(format!("execute {program} failed: {err}")))?;
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    log.event(json!({
        "event": "command_finish",
        "program": program,
        "status": output.status.code(),
        "stdout": stdout,
        "stderr": stderr,
    }))?;
    if output.status.success() {
        return Ok(());
    }
    let stderr_tail = stderr
        .lines()
        .rev()
        .filter(|line| !line.trim().is_empty())
        .take(12)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join("\n");
    let stdout_tail = stdout
        .lines()
        .rev()
        .filter(|line| !line.trim().is_empty())
        .take(6)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join("\n");
    Err(CliError::from(CalyxError::lens_unreachable(format!(
        "{program} exited with {:?}; stderr_tail={}{}",
        output.status.code(),
        if stderr_tail.is_empty() {
            "no stderr"
        } else {
            &stderr_tail
        },
        if stdout_tail.is_empty() {
            String::new()
        } else {
            format!("\nstdout_tail={stdout_tail}")
        }
    ))))
}

pub(super) fn write_json_file(path: &Path, value: &impl Serialize) -> CliResult {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let bytes = serde_json::to_vec_pretty(value)?;
    let mut file = File::create(path)?;
    file.write_all(&bytes)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    Ok(())
}
