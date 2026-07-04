use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::Subcommand;
use super::vault::home_dir;

use crate::durable_write::write_json_value_atomic;
use crate::error::{CliError, CliResult};
use crate::fsv_vault_health_quarantine::{
    QUARANTINE_FILE, QUARANTINE_INVALID_CODE, QUARANTINE_SCHEMA, sha256_hex,
};
use crate::output::print_json;
mod support;
use support::{index_path, now_ms, relative_to_home, retire_error};
mod index;
mod supersession;

use index::{
    active_position, active_vault_count, push_retired_record, read_index_value, required_entry_str,
    retired_record, retired_vault_count, vaults_array_mut, verify_retirement_readback,
};
const RETIREMENT_SCHEMA: &str = "calyx.vault_retirement.v1";
const RETIREMENT_SOURCE_OF_TRUTH: &str =
    "vaults/index.json retired_vaults record plus absence from the active vaults array";
const NOT_QUARANTINED_CODE: &str = "CALYX_VAULT_RETIREMENT_NOT_QUARANTINED";
const NOT_ACTIVE_CODE: &str = "CALYX_VAULT_RETIREMENT_NOT_ACTIVE";
const ALREADY_RETIRED_CODE: &str = "CALYX_VAULT_ALREADY_RETIRED";
const INDEX_CORRUPT_CODE: &str = "CALYX_VAULT_INDEX_CORRUPT";
const READBACK_MISMATCH_CODE: &str = "CALYX_VAULT_RETIREMENT_READBACK_MISMATCH";
const SOURCE_MISSING_CODE: &str = "CALYX_VAULT_RETIREMENT_SOURCE_MISSING";
const CLOCK_FAILED_CODE: &str = "CALYX_VAULT_RETIREMENT_CLOCK_FAILED";
const REMEDIATION: &str = "inspect vaults/index.json, the vault CURRENT/manifest files, and fsv_quarantine.json before retrying retirement";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RetireVaultArgs {
    pub(crate) vault: String,
    pub(crate) reason: String,
    pub(crate) superseded_by: Option<String>,
    pub(crate) source_issue: Option<String>,
    pub(crate) fsv_readback: Option<PathBuf>,
    pub(crate) fsv_sha256: Option<String>,
}

impl RetireVaultArgs {
    fn supersession_requested(&self) -> bool {
        self.superseded_by.is_some()
            || self.source_issue.is_some()
            || self.fsv_readback.is_some()
            || self.fsv_sha256.is_some()
    }
}

#[derive(Clone, Debug, Serialize)]
struct RetireVaultReport {
    status: &'static str,
    schema: &'static str,
    source_of_truth: &'static str,
    index_path: String,
    index_before_sha256: String,
    index_after_sha256: String,
    active_vault_count_before: usize,
    active_vault_count_after: usize,
    retired_vault_count_after: usize,
    vault_id: String,
    vault_name: String,
    vault_path: String,
    retirement_record: VaultRetirementRecord,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
struct VaultRetirementRecord {
    schema: String,
    source_of_truth: String,
    vault_id: String,
    name: String,
    path: String,
    panel_template: Option<String>,
    reason: String,
    retired_at_unix_ms: u64,
    retired_by: String,
    active_index_before_sha256: String,
    active_index_before_vault_count: usize,
    original_index_entry: Value,
    current_pointer: FileEvidence,
    current_manifest: FileEvidence,
    manifest_sequence: Option<u64>,
    durable_sequence: Option<u64>,
    manifest_registry_ref: Value,
    registry_snapshot_file_count: usize,
    registry_snapshot_files: Vec<FileEvidence>,
    quarantine_marker: FileEvidence,
    quarantine_failed_checks: Value,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct FileEvidence {
    path: String,
    sha256: String,
    bytes: u64,
}

pub(crate) fn parse_retire_vault(rest: &[String]) -> CliResult<Subcommand> {
    let vault = rest
        .first()
        .ok_or_else(|| {
            CliError::usage(
                "retire-vault requires <vault> --reason <text> [--superseded-by <vault> --source-issue <issue> --fsv-readback <json> --fsv-sha256 <sha256>]",
            )
        })?
        .clone();
    let mut reason = None;
    let mut superseded_by = None;
    let mut source_issue = None;
    let mut fsv_readback = None;
    let mut fsv_sha256 = None;
    let mut index = 1;
    while index < rest.len() {
        match rest[index].as_str() {
            "--reason" => {
                index += 1;
                reason = Some(
                    rest.get(index)
                        .ok_or_else(|| CliError::usage("--reason requires a value"))?
                        .clone(),
                );
            }
            "--superseded-by" | "--replacement-vault" => {
                index += 1;
                superseded_by = Some(
                    rest.get(index)
                        .ok_or_else(|| CliError::usage("--superseded-by requires a value"))?
                        .clone(),
                );
            }
            "--source-issue" => {
                index += 1;
                source_issue = Some(
                    rest.get(index)
                        .ok_or_else(|| CliError::usage("--source-issue requires a value"))?
                        .clone(),
                );
            }
            "--fsv-readback" => {
                index += 1;
                fsv_readback =
                    Some(PathBuf::from(rest.get(index).ok_or_else(|| {
                        CliError::usage("--fsv-readback requires a value")
                    })?));
            }
            "--fsv-sha256" => {
                index += 1;
                fsv_sha256 = Some(
                    rest.get(index)
                        .ok_or_else(|| CliError::usage("--fsv-sha256 requires a value"))?
                        .clone(),
                );
            }
            other => {
                return Err(CliError::usage(format!(
                    "unexpected retire-vault flag {other}"
                )));
            }
        }
        index += 1;
    }
    let reason = reason.ok_or_else(|| CliError::usage("retire-vault requires --reason <text>"))?;
    if reason.trim().is_empty() {
        return Err(CliError::usage("retire-vault --reason must not be empty"));
    }
    let supersession_flag_count = [
        superseded_by.is_some(),
        source_issue.is_some(),
        fsv_readback.is_some(),
        fsv_sha256.is_some(),
    ]
    .into_iter()
    .filter(|present| *present)
    .count();
    if supersession_flag_count != 0 && supersession_flag_count != 4 {
        return Err(CliError::usage(
            "retire-vault supersession requires --superseded-by, --source-issue, --fsv-readback, and --fsv-sha256 together",
        ));
    }
    for (name, value) in [
        ("--superseded-by", superseded_by.as_deref()),
        ("--source-issue", source_issue.as_deref()),
        ("--fsv-sha256", fsv_sha256.as_deref()),
    ] {
        if value.is_some_and(|value| value.trim().is_empty()) {
            return Err(CliError::usage(format!(
                "retire-vault {name} must not be empty"
            )));
        }
    }
    if fsv_readback
        .as_ref()
        .is_some_and(|path| path.as_os_str().is_empty())
    {
        return Err(CliError::usage(
            "retire-vault --fsv-readback must not be empty",
        ));
    }
    Ok(Subcommand::RetireVault(RetireVaultArgs {
        vault,
        reason,
        superseded_by,
        source_issue,
        fsv_readback,
        fsv_sha256,
    }))
}

pub(crate) fn run(args: RetireVaultArgs) -> CliResult {
    let home = home_dir()?;
    run_with_home(&home, args)
}

fn run_with_home(home: &Path, args: RetireVaultArgs) -> CliResult {
    if args.supersession_requested() {
        return supersession::run_with_home(home, args);
    }

    let index_path = index_path(home);
    let (mut index, before_bytes) = read_index_value(&index_path)?;
    let before_hash = sha256_hex(&before_bytes);
    let before_count = active_vault_count(&index)?;
    let active_pos = active_position(&index, &args.vault)?;

    if active_pos.is_none() {
        if let Some(record) = retired_record(&index, &args.vault)? {
            return Err(retire_error(
                ALREADY_RETIRED_CODE,
                format!(
                    "vault {} was already retired at {} with reason {}",
                    record["vault_id"].as_str().unwrap_or(&args.vault),
                    record["retired_at_unix_ms"].as_u64().unwrap_or_default(),
                    record["reason"].as_str().unwrap_or("<missing>")
                ),
            ));
        }
        return Err(retire_error(
            NOT_ACTIVE_CODE,
            format!(
                "vault {} is not present in the active vault index",
                args.vault
            ),
        ));
    }

    let vaults = vaults_array_mut(&mut index)?;
    let original_entry = vaults.remove(active_pos.expect("checked active position"));
    let vault_id = required_entry_str(&original_entry, "vault_id")?.to_string();
    let name = required_entry_str(&original_entry, "name")?.to_string();
    let relative_path = required_entry_str(&original_entry, "path")?.to_string();
    let panel_template = original_entry
        .get("panel_template")
        .and_then(Value::as_str)
        .map(str::to_string);
    let vault_dir = home.join(&relative_path);
    let record = build_record(
        home,
        &vault_dir,
        &original_entry,
        &RecordIdentity {
            vault_id,
            name,
            relative_path,
            panel_template,
        },
        &args.reason,
        &before_hash,
        before_count,
    )?;

    push_retired_record(&mut index, &record)?;
    write_json_value_atomic(&index_path, &index, "vault retirement index")?;
    let (after_index, after_bytes) = read_index_value(&index_path)?;
    verify_retirement_readback(&after_index, &record)?;

    print_json(&RetireVaultReport {
        status: "retired",
        schema: RETIREMENT_SCHEMA,
        source_of_truth: RETIREMENT_SOURCE_OF_TRUTH,
        index_path: index_path.display().to_string(),
        index_before_sha256: before_hash,
        index_after_sha256: sha256_hex(&after_bytes),
        active_vault_count_before: before_count,
        active_vault_count_after: active_vault_count(&after_index)?,
        retired_vault_count_after: retired_vault_count(&after_index)?,
        vault_id: record.vault_id.clone(),
        vault_name: record.name.clone(),
        vault_path: record.path.clone(),
        retirement_record: record,
    })
}

struct RecordIdentity {
    vault_id: String,
    name: String,
    relative_path: String,
    panel_template: Option<String>,
}

fn build_record(
    home: &Path,
    vault_dir: &Path,
    original_entry: &Value,
    identity: &RecordIdentity,
    reason: &str,
    before_hash: &str,
    before_count: usize,
) -> CliResult<VaultRetirementRecord> {
    if !vault_dir.is_dir() {
        return Err(retire_error(
            SOURCE_MISSING_CODE,
            format!("vault directory {} is missing", vault_dir.display()),
        ));
    }
    let (current_pointer, current_value) = current_pointer(home, vault_dir)?;
    let current_ref = current_value.trim();
    if current_ref.is_empty() {
        return Err(retire_error(
            SOURCE_MISSING_CODE,
            format!("CURRENT in {} is empty", vault_dir.display()),
        ));
    }
    let manifest_path = vault_dir.join(current_ref);
    let (current_manifest, manifest_bytes) = file_evidence(home, &manifest_path)?;
    let manifest: Value = serde_json::from_slice(&manifest_bytes).map_err(|error| {
        retire_error(
            SOURCE_MISSING_CODE,
            format!(
                "current manifest {} is not valid JSON: {error}",
                manifest_path.display()
            ),
        )
    })?;
    let quarantine_path = vault_dir.join(QUARANTINE_FILE);
    let (quarantine_marker, quarantine_value) =
        quarantine_evidence(home, &quarantine_path, identity)?;
    let registry_snapshot_files = registry_file_evidence(home, vault_dir)?;
    Ok(VaultRetirementRecord {
        schema: RETIREMENT_SCHEMA.to_string(),
        source_of_truth: RETIREMENT_SOURCE_OF_TRUTH.to_string(),
        vault_id: identity.vault_id.clone(),
        name: identity.name.clone(),
        path: identity.relative_path.clone(),
        panel_template: identity.panel_template.clone(),
        reason: reason.to_string(),
        retired_at_unix_ms: now_ms()?,
        retired_by: "calyx retire-vault".to_string(),
        active_index_before_sha256: before_hash.to_string(),
        active_index_before_vault_count: before_count,
        original_index_entry: original_entry.clone(),
        current_pointer,
        current_manifest,
        manifest_sequence: manifest.get("manifest_seq").and_then(Value::as_u64),
        durable_sequence: manifest.get("durable_seq").and_then(Value::as_u64),
        manifest_registry_ref: manifest.get("registry_ref").cloned().unwrap_or(Value::Null),
        registry_snapshot_file_count: registry_snapshot_files.len(),
        registry_snapshot_files,
        quarantine_marker,
        quarantine_failed_checks: quarantine_value
            .get("failed_checks")
            .cloned()
            .unwrap_or(Value::Null),
    })
}

pub(super) fn current_pointer(home: &Path, vault_dir: &Path) -> CliResult<(FileEvidence, String)> {
    let path = vault_dir.join("CURRENT");
    let (evidence, bytes) = file_evidence(home, &path)?;
    let value = String::from_utf8(bytes).map_err(|error| {
        retire_error(
            SOURCE_MISSING_CODE,
            format!("CURRENT {} is not UTF-8: {error}", path.display()),
        )
    })?;
    Ok((evidence, value))
}

fn quarantine_evidence(
    home: &Path,
    path: &Path,
    identity: &RecordIdentity,
) -> CliResult<(FileEvidence, Value)> {
    if !path.exists() {
        return Err(retire_error(
            NOT_QUARANTINED_CODE,
            format!(
                "vault {} cannot be retired without physical {}",
                identity.vault_id,
                path.display()
            ),
        ));
    }
    let (evidence, bytes) = file_evidence(home, path)?;
    let value: Value = serde_json::from_slice(&bytes).map_err(|error| {
        retire_error(
            QUARANTINE_INVALID_CODE,
            format!(
                "quarantine marker {} is not valid JSON: {error}",
                path.display()
            ),
        )
    })?;
    if value.get("schema").and_then(Value::as_str) != Some(QUARANTINE_SCHEMA) {
        return Err(retire_error(
            QUARANTINE_INVALID_CODE,
            format!("quarantine marker {} has the wrong schema", path.display()),
        ));
    }
    if value.get("vault_id").and_then(Value::as_str) != Some(identity.vault_id.as_str()) {
        return Err(retire_error(
            QUARANTINE_INVALID_CODE,
            format!(
                "quarantine marker {} vault_id does not match {}",
                path.display(),
                identity.vault_id
            ),
        ));
    }
    let failed = value
        .get("failed_checks")
        .and_then(Value::as_array)
        .is_some_and(|checks| !checks.is_empty());
    if !failed {
        return Err(retire_error(
            QUARANTINE_INVALID_CODE,
            format!("quarantine marker {} has no failed_checks", path.display()),
        ));
    }
    Ok((evidence, value))
}

fn registry_file_evidence(home: &Path, vault_dir: &Path) -> CliResult<Vec<FileEvidence>> {
    let registry = vault_dir.join("registry");
    if !registry.exists() {
        return Ok(Vec::new());
    }
    if !registry.is_dir() {
        return Err(retire_error(
            SOURCE_MISSING_CODE,
            format!("registry path {} is not a directory", registry.display()),
        ));
    }
    let mut paths = fs::read_dir(&registry)?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<Result<Vec<_>, _>>()?;
    paths.sort();
    paths
        .into_iter()
        .filter(|path| path.is_file())
        .map(|path| file_evidence(home, &path).map(|(evidence, _)| evidence))
        .collect()
}

pub(super) fn file_evidence(home: &Path, path: &Path) -> CliResult<(FileEvidence, Vec<u8>)> {
    let bytes = fs::read(path).map_err(|error| {
        retire_error(
            SOURCE_MISSING_CODE,
            format!(
                "read source-of-truth file {} failed: {error}",
                path.display()
            ),
        )
    })?;
    Ok((
        FileEvidence {
            path: relative_to_home(home, path),
            sha256: sha256_hex(&bytes),
            bytes: bytes.len() as u64,
        },
        bytes,
    ))
}

#[cfg(test)]
mod tests;
