use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use calyx_core::CalyxError;
use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::durable_write::write_json_value_atomic;
use crate::error::{CliError, CliResult};
use crate::fsv_vault_health::{REMEDIATION, VaultHealthCheck, VaultHealthReport, failed};

pub(crate) const QUARANTINE_SCHEMA: &str = "calyx.fsv.vault_quarantine.v1";
pub(crate) const QUARANTINE_FILE: &str = "fsv_quarantine.json";
pub(crate) const QUARANTINED_CODE: &str = "CALYX_FSV_VAULT_QUARANTINED";
pub(crate) const QUARANTINE_INVALID_CODE: &str = "CALYX_FSV_VAULT_QUARANTINE_INVALID";

#[derive(Clone, Debug, Serialize)]
struct FailedCheckMarker {
    name: &'static str,
    code: String,
    message: String,
    remediation: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
struct QuarantineMarker<'a> {
    schema: &'static str,
    source_of_truth: &'static str,
    vault_id: &'a str,
    vault_name: &'a str,
    vault_dir: &'a str,
    written_at_unix_ms: u64,
    failed_checks: Vec<FailedCheckMarker>,
}

pub(crate) struct QuarantineWrite {
    pub(crate) sha256_hex: String,
}

pub(crate) fn check_marker(path: &Path) -> VaultHealthCheck {
    if !path.exists() {
        return failed_ok(
            "quarantine_marker",
            "no FSV quarantine marker is present",
            json!({"path": path.display().to_string(), "present": false}),
        );
    }
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) => {
            return failed(
                "quarantine_marker",
                QUARANTINE_INVALID_CODE,
                format!("read quarantine marker {} failed: {error}", path.display()),
                "inspect or remove the unreadable quarantine marker only after restoring the real vault source of truth",
                json!({"path": path.display().to_string()}),
            );
        }
    };
    match serde_json::from_slice::<Value>(&bytes) {
        Ok(value) if value.get("schema").and_then(Value::as_str) == Some(QUARANTINE_SCHEMA) => {
            failed(
                "quarantine_marker",
                QUARANTINED_CODE,
                format!(
                    "vault has an active FSV quarantine marker at {}",
                    path.display()
                ),
                REMEDIATION,
                json!({
                    "path": path.display().to_string(),
                    "present": true,
                    "sha256_hex": sha256_hex(&bytes),
                    "failed_checks": value.get("failed_checks").cloned().unwrap_or(Value::Null)
                }),
            )
        }
        Ok(value) => failed(
            "quarantine_marker",
            QUARANTINE_INVALID_CODE,
            format!("quarantine marker {} has an invalid schema", path.display()),
            "replace the marker only through `calyx fsv vault-health --write-quarantine` after inspecting the vault",
            json!({"path": path.display().to_string(), "decoded": value}),
        ),
        Err(error) => failed(
            "quarantine_marker",
            QUARANTINE_INVALID_CODE,
            format!(
                "quarantine marker {} is not valid JSON: {error}",
                path.display()
            ),
            "replace the marker only through `calyx fsv vault-health --write-quarantine` after inspecting the vault",
            json!({"path": path.display().to_string(), "sha256_hex": sha256_hex(&bytes)}),
        ),
    }
}

pub(crate) fn write_marker(report: &VaultHealthReport) -> CliResult<QuarantineWrite> {
    let marker_path = PathBuf::from(&report.quarantine_marker_path);
    let failed_checks = report
        .checks
        .iter()
        .filter(|check| check.status != "ok")
        .map(|check| FailedCheckMarker {
            name: check.name,
            code: check
                .code
                .clone()
                .unwrap_or_else(|| format!("{}:failed", check.name)),
            message: check.message.clone(),
            remediation: check.remediation.clone(),
        })
        .collect::<Vec<_>>();
    let marker = QuarantineMarker {
        schema: QUARANTINE_SCHEMA,
        source_of_truth: "this physical quarantine marker was atomically written and read back from the vault directory",
        vault_id: &report.vault_id,
        vault_name: &report.vault_name,
        vault_dir: &report.vault_dir,
        written_at_unix_ms: now_ms(),
        failed_checks,
    };
    let value = serde_json::to_value(marker)?;
    write_json_value_atomic(&marker_path, &value, "fsv quarantine marker")?;
    let bytes = fs::read(&marker_path)?;
    let decoded: Value = serde_json::from_slice(&bytes)?;
    if decoded.get("schema").and_then(Value::as_str) != Some(QUARANTINE_SCHEMA) {
        return Err(CliError::Calyx(CalyxError {
            code: QUARANTINE_INVALID_CODE,
            message: format!(
                "wrote quarantine marker {}, but readback schema did not match {QUARANTINE_SCHEMA}",
                marker_path.display()
            ),
            remediation: "inspect the quarantine marker file and rerun vault-health",
        }));
    }
    if decoded.get("vault_id").and_then(Value::as_str) != Some(report.vault_id.as_str()) {
        return Err(CliError::Calyx(CalyxError {
            code: QUARANTINE_INVALID_CODE,
            message: format!(
                "wrote quarantine marker {}, but readback vault_id did not match {}",
                marker_path.display(),
                report.vault_id
            ),
            remediation: "inspect the quarantine marker file and rerun vault-health",
        }));
    }
    Ok(QuarantineWrite {
        sha256_hex: sha256_hex(&bytes),
    })
}

fn failed_ok(name: &'static str, message: impl Into<String>, details: Value) -> VaultHealthCheck {
    VaultHealthCheck {
        name,
        status: "ok",
        code: None,
        message: message.into(),
        remediation: None,
        details,
    }
}

pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_millis() as u64)
}
