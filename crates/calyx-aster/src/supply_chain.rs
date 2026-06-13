//! Supply-chain integrity checks for PH61.

use std::fs;
use std::path::Path;
use std::process::Command;

use calyx_core::{CalyxError, LensId, Result, Timestamp, content_address};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use toml::Value as TomlValue;

/// Module-local error for Cargo.lock/SBOM parse failures.
pub const CALYX_SBOM_PARSE_ERROR: &str = "CALYX_SBOM_PARSE_ERROR";
/// Module-local error for actionable dependency vulnerabilities.
pub const CALYX_SUPPLY_CHAIN_VULN: &str = "CALYX_SUPPLY_CHAIN_VULN";
/// Module-local error for swapped or unreadable lens weights.
pub const CALYX_LENS_WEIGHT_TAMPERED: &str = "CALYX_LENS_WEIGHT_TAMPERED";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SbomEntry {
    pub crate_name: String,
    pub version: String,
    pub checksum: Option<String>,
    pub source: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sbom {
    pub generated_at: Timestamp,
    pub entries: Vec<SbomEntry>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CargoAuditResult {
    Clean,
    Vulnerabilities(Vec<AuditVulnEntry>),
    ToolNotFound,
    ParseError(String),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditVulnEntry {
    pub package: String,
    pub version: String,
    pub advisory_id: String,
    pub description: String,
}

pub fn generate_sbom(cargo_lock_path: &Path, now: Timestamp) -> Result<Sbom> {
    let text = fs::read_to_string(cargo_lock_path).map_err(|error| {
        sbom_parse_error(format!("read {}: {error}", cargo_lock_path.display()))
    })?;
    let document = text.parse::<TomlValue>().map_err(|error| {
        sbom_parse_error(format!("parse {}: {error}", cargo_lock_path.display()))
    })?;
    let Some(packages) = document.get("package").and_then(TomlValue::as_array) else {
        return Ok(Sbom {
            generated_at: now,
            entries: Vec::new(),
        });
    };

    let mut entries = Vec::with_capacity(packages.len());
    for package in packages {
        let Some(crate_name) = package.get("name").and_then(TomlValue::as_str) else {
            eprintln!("calyx supply-chain skipped Cargo.lock package without string name");
            continue;
        };
        let Some(version) = package.get("version").and_then(TomlValue::as_str) else {
            eprintln!(
                "calyx supply-chain skipped Cargo.lock package {crate_name} without string version"
            );
            continue;
        };
        let source = package
            .get("source")
            .and_then(TomlValue::as_str)
            .unwrap_or_default()
            .to_string();
        let checksum = package
            .get("checksum")
            .and_then(TomlValue::as_str)
            .map(ToString::to_string);
        entries.push(SbomEntry {
            crate_name: crate_name.to_string(),
            version: version.to_string(),
            checksum,
            source,
        });
    }
    Ok(Sbom {
        generated_at: now,
        entries,
    })
}

pub fn run_cargo_audit(cargo_lock_path: &Path) -> CargoAuditResult {
    run_cargo_audit_with_command(Path::new("cargo"), cargo_lock_path)
}

fn run_cargo_audit_with_command(cargo_command: &Path, cargo_lock_path: &Path) -> CargoAuditResult {
    let output = match Command::new(cargo_command)
        .arg("audit")
        .arg("--json")
        .arg("--file")
        .arg(cargo_lock_path)
        .output()
    {
        Ok(output) => output,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return CargoAuditResult::ToolNotFound;
        }
        Err(error) => return CargoAuditResult::ParseError(error.to_string()),
    };

    let stderr = String::from_utf8_lossy(&output.stderr);
    if cargo_audit_missing(&stderr) {
        return CargoAuditResult::ToolNotFound;
    }
    if output.status.success() {
        return parse_audit_json(&output.stdout)
            .map(|_| CargoAuditResult::Clean)
            .unwrap_or_else(CargoAuditResult::ParseError);
    }
    match parse_audit_json(&output.stdout) {
        Ok(vulnerabilities) if !vulnerabilities.is_empty() => {
            CargoAuditResult::Vulnerabilities(vulnerabilities)
        }
        Ok(_) => CargoAuditResult::ParseError(stderr.to_string()),
        Err(error) => CargoAuditResult::ParseError(error),
    }
}

pub fn assert_audit_clean(result: &CargoAuditResult) -> Result<()> {
    match result {
        CargoAuditResult::Clean | CargoAuditResult::ToolNotFound => Ok(()),
        CargoAuditResult::Vulnerabilities(vulnerabilities) => Err(supply_chain_vuln(format!(
            "cargo audit reported {} vulnerabilities",
            vulnerabilities.len()
        ))),
        CargoAuditResult::ParseError(error) => Err(sbom_parse_error(format!(
            "cargo audit JSON parse failed: {error}"
        ))),
    }
}

pub fn lens_weight_content_address(bytes: &[u8]) -> [u8; 16] {
    let digest = blake3::hash(bytes);
    content_address([digest.as_bytes().as_slice()])
}

pub fn verify_lens_weight_hash(lens_id: &LensId, weights_path: &Path) -> Result<()> {
    let weights = fs::read(weights_path).map_err(|error| {
        lens_weight_tampered(format!("read {}: {error}", weights_path.display()))
    })?;
    let actual = lens_weight_content_address(&weights);
    if lens_id.as_bytes() == &actual {
        Ok(())
    } else {
        Err(lens_weight_tampered(format!(
            "lens weights at {} do not match registered LensId {}",
            weights_path.display(),
            lens_id
        )))
    }
}

fn parse_audit_json(stdout: &[u8]) -> std::result::Result<Vec<AuditVulnEntry>, String> {
    if stdout.is_empty() {
        return Ok(Vec::new());
    }
    let value: JsonValue = serde_json::from_slice(stdout).map_err(|error| error.to_string())?;
    let Some(list) = value
        .pointer("/vulnerabilities/list")
        .and_then(JsonValue::as_array)
    else {
        return Ok(Vec::new());
    };
    Ok(list.iter().map(audit_entry_from_json).collect())
}

fn audit_entry_from_json(value: &JsonValue) -> AuditVulnEntry {
    let package = value
        .pointer("/package/name")
        .and_then(JsonValue::as_str)
        .unwrap_or_default()
        .to_string();
    let version = value
        .pointer("/package/version")
        .and_then(JsonValue::as_str)
        .unwrap_or_default()
        .to_string();
    let advisory_id = value
        .pointer("/advisory/id")
        .and_then(JsonValue::as_str)
        .unwrap_or_default()
        .to_string();
    let description = value
        .pointer("/advisory/title")
        .or_else(|| value.pointer("/advisory/description"))
        .and_then(JsonValue::as_str)
        .unwrap_or_default()
        .to_string();
    AuditVulnEntry {
        package,
        version,
        advisory_id,
        description,
    }
}

fn cargo_audit_missing(stderr: &str) -> bool {
    let stderr = stderr.to_ascii_lowercase();
    stderr.contains("no such command")
        || stderr.contains("no such subcommand")
        || (stderr.contains("audit") && stderr.contains("not installed"))
        || (stderr.contains("audit") && stderr.contains("not found"))
}

fn sbom_parse_error(message: impl Into<String>) -> CalyxError {
    supply_chain_error(
        CALYX_SBOM_PARSE_ERROR,
        message,
        "fix Cargo.lock or cargo audit JSON before generating the SBOM",
    )
}

fn supply_chain_vuln(message: impl Into<String>) -> CalyxError {
    supply_chain_error(
        CALYX_SUPPLY_CHAIN_VULN,
        message,
        "upgrade, patch, or explicitly quarantine the vulnerable dependency",
    )
}

fn lens_weight_tampered(message: impl Into<String>) -> CalyxError {
    supply_chain_error(
        CALYX_LENS_WEIGHT_TAMPERED,
        message,
        "treat the weights as a different LensId and re-register before use",
    )
}

fn supply_chain_error(
    code: &'static str,
    message: impl Into<String>,
    remediation: &'static str,
) -> CalyxError {
    CalyxError {
        code,
        message: message.into(),
        remediation,
    }
}

#[cfg(test)]
mod tests;
