use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::index::{
    active_position, active_vault_count, read_index_value, required_entry_str, vaults_array,
    vaults_array_mut,
};
use super::support::{index_path, now_ms, retire_error};
use super::{
    FileEvidence, INDEX_CORRUPT_CODE, NOT_ACTIVE_CODE, READBACK_MISMATCH_CODE, RetireVaultArgs,
    SOURCE_MISSING_CODE, current_pointer, file_evidence,
};
use crate::durable_write::write_json_value_atomic;
use crate::error::{CliError, CliResult};
use crate::fsv_vault_health_quarantine::sha256_hex;
use crate::output::print_json;

const SUPERSESSION_SCHEMA: &str = "calyx.vault_supersession.v1";
const SUPERSESSION_SOURCE_OF_TRUTH: &str =
    "vaults/index.json superseded_vaults record plus replacement active readback";
pub(super) const ALREADY_SUPERSEDED_CODE: &str = "CALYX_VAULT_ALREADY_SUPERSEDED";
pub(super) const REPLACEMENT_NOT_ACTIVE_CODE: &str =
    "CALYX_VAULT_SUPERSESSION_REPLACEMENT_NOT_ACTIVE";
pub(super) const FSV_MISMATCH_CODE: &str = "CALYX_VAULT_SUPERSESSION_FSV_MISMATCH";

#[derive(Clone, Debug, Serialize)]
struct SupersedeVaultReport {
    status: &'static str,
    schema: &'static str,
    source_of_truth: &'static str,
    index_path: String,
    index_before_sha256: String,
    index_after_sha256: String,
    active_vault_count_before: usize,
    active_vault_count_after: usize,
    superseded_vault_count_after: usize,
    superseded_vault_id: String,
    superseded_vault_name: String,
    replacement_vault_id: String,
    replacement_vault_name: String,
    supersession_record: VaultSupersessionRecord,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
struct VaultSupersessionRecord {
    schema: String,
    source_of_truth: String,
    superseded_vault_id: String,
    superseded_name: String,
    superseded_path: String,
    superseded_panel_template: Option<String>,
    replacement_vault_id: String,
    replacement_name: String,
    replacement_path: String,
    replacement_panel_template: Option<String>,
    reason: String,
    source_issue: String,
    superseded_at_unix_ms: u64,
    superseded_by_command: String,
    active_index_before_sha256: String,
    active_index_before_vault_count: usize,
    superseded_original_index_entry: Value,
    replacement_index_entry: Value,
    superseded_current_pointer: FileEvidence,
    superseded_current_manifest: FileEvidence,
    replacement_current_pointer: FileEvidence,
    replacement_current_manifest: FileEvidence,
    fsv_readback_artifact: FileEvidence,
    fsv_readback_expected_sha256: String,
    fsv_matched_replacement_vault_id: bool,
    fsv_matched_replacement_name: bool,
}

#[derive(Clone, Debug)]
struct ActiveVaultIdentity {
    vault_id: String,
    name: String,
    path: String,
    panel_template: Option<String>,
    original_entry: Value,
}

pub(super) fn run_with_home(home: &Path, args: RetireVaultArgs) -> CliResult {
    let replacement_ref = args
        .superseded_by
        .as_ref()
        .expect("parser requires replacement for supersession");
    let index_path = index_path(home);
    let (mut index, before_bytes) = read_index_value(&index_path)?;
    let before_hash = sha256_hex(&before_bytes);
    let before_count = active_vault_count(&index)?;
    let superseded_pos = match active_position(&index, &args.vault)? {
        Some(pos) => pos,
        None => {
            if let Some(record) = superseded_record(&index, &args.vault)? {
                return Err(retire_error(
                    ALREADY_SUPERSEDED_CODE,
                    format!(
                        "vault {} was already superseded by {} at {}",
                        record["superseded_vault_id"]
                            .as_str()
                            .unwrap_or(args.vault.as_str()),
                        record["replacement_vault_id"]
                            .as_str()
                            .unwrap_or("<missing>"),
                        record["superseded_at_unix_ms"].as_u64().unwrap_or_default()
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
    };
    let replacement_pos = active_position(&index, replacement_ref)?.ok_or_else(|| {
        retire_error(
            REPLACEMENT_NOT_ACTIVE_CODE,
            format!("replacement vault {replacement_ref} is not active"),
        )
    })?;
    if replacement_pos == superseded_pos {
        return Err(retire_error(
            REPLACEMENT_NOT_ACTIVE_CODE,
            "superseded vault and replacement vault must be different active entries",
        ));
    }

    let (superseded_entry, replacement_entry) = {
        let vaults = vaults_array(&index)?;
        (
            vaults[superseded_pos].clone(),
            vaults[replacement_pos].clone(),
        )
    };
    let superseded = identity_from_entry(&superseded_entry)?;
    let replacement = identity_from_entry(&replacement_entry)?;
    let record = build_record(
        home,
        &superseded,
        &replacement,
        &args,
        &before_hash,
        before_count,
    )?;

    vaults_array_mut(&mut index)?.remove(superseded_pos);
    push_superseded_record(&mut index, &record)?;
    write_json_value_atomic(&index_path, &index, "vault supersession index")?;
    let (after_index, after_bytes) = read_index_value(&index_path)?;
    verify_supersession_readback(&after_index, &record)?;

    print_json(&SupersedeVaultReport {
        status: "superseded",
        schema: SUPERSESSION_SCHEMA,
        source_of_truth: SUPERSESSION_SOURCE_OF_TRUTH,
        index_path: index_path.display().to_string(),
        index_before_sha256: before_hash,
        index_after_sha256: sha256_hex(&after_bytes),
        active_vault_count_before: before_count,
        active_vault_count_after: active_vault_count(&after_index)?,
        superseded_vault_count_after: superseded_vault_count(&after_index)?,
        superseded_vault_id: record.superseded_vault_id.clone(),
        superseded_vault_name: record.superseded_name.clone(),
        replacement_vault_id: record.replacement_vault_id.clone(),
        replacement_vault_name: record.replacement_name.clone(),
        supersession_record: record,
    })
}

fn build_record(
    home: &Path,
    superseded: &ActiveVaultIdentity,
    replacement: &ActiveVaultIdentity,
    args: &RetireVaultArgs,
    before_hash: &str,
    before_count: usize,
) -> CliResult<VaultSupersessionRecord> {
    let (superseded_current_pointer, superseded_current_manifest) =
        vault_manifest_evidence(home, superseded)?;
    let (replacement_current_pointer, replacement_current_manifest) =
        vault_manifest_evidence(home, replacement)?;
    let fsv_path = args
        .fsv_readback
        .as_ref()
        .expect("parser requires fsv readback for supersession");
    let expected_sha = normalize_sha256(
        args.fsv_sha256
            .as_deref()
            .expect("parser requires fsv sha for supersession"),
    )?;
    let (fsv_readback_artifact, fsv_value) =
        fsv_readback_evidence(home, fsv_path, &expected_sha, replacement)?;
    let fsv_matched_replacement_name = value_contains_string(&fsv_value, &replacement.name);
    Ok(VaultSupersessionRecord {
        schema: SUPERSESSION_SCHEMA.to_string(),
        source_of_truth: SUPERSESSION_SOURCE_OF_TRUTH.to_string(),
        superseded_vault_id: superseded.vault_id.clone(),
        superseded_name: superseded.name.clone(),
        superseded_path: superseded.path.clone(),
        superseded_panel_template: superseded.panel_template.clone(),
        replacement_vault_id: replacement.vault_id.clone(),
        replacement_name: replacement.name.clone(),
        replacement_path: replacement.path.clone(),
        replacement_panel_template: replacement.panel_template.clone(),
        reason: args.reason.clone(),
        source_issue: args
            .source_issue
            .clone()
            .expect("parser requires source issue for supersession"),
        superseded_at_unix_ms: now_ms()?,
        superseded_by_command: "calyx retire-vault --superseded-by".to_string(),
        active_index_before_sha256: before_hash.to_string(),
        active_index_before_vault_count: before_count,
        superseded_original_index_entry: superseded.original_entry.clone(),
        replacement_index_entry: replacement.original_entry.clone(),
        superseded_current_pointer,
        superseded_current_manifest,
        replacement_current_pointer,
        replacement_current_manifest,
        fsv_readback_artifact,
        fsv_readback_expected_sha256: expected_sha,
        fsv_matched_replacement_vault_id: true,
        fsv_matched_replacement_name,
    })
}

fn vault_manifest_evidence(
    home: &Path,
    identity: &ActiveVaultIdentity,
) -> CliResult<(FileEvidence, FileEvidence)> {
    let vault_dir = resolve_home_path(home, &identity.path);
    if !vault_dir.is_dir() {
        return Err(retire_error(
            SOURCE_MISSING_CODE,
            format!(
                "vault directory {} for {} is missing",
                vault_dir.display(),
                identity.vault_id
            ),
        ));
    }
    let (current_pointer, current_value) = current_pointer(home, &vault_dir)?;
    let current_ref = current_value.trim();
    if current_ref.is_empty() {
        return Err(retire_error(
            SOURCE_MISSING_CODE,
            format!("CURRENT in {} is empty", vault_dir.display()),
        ));
    }
    let manifest_path = vault_dir.join(current_ref);
    let (current_manifest, manifest_bytes) = file_evidence(home, &manifest_path)?;
    serde_json::from_slice::<Value>(&manifest_bytes).map_err(|error| {
        retire_error(
            SOURCE_MISSING_CODE,
            format!(
                "current manifest {} is not valid JSON: {error}",
                manifest_path.display()
            ),
        )
    })?;
    Ok((current_pointer, current_manifest))
}

fn fsv_readback_evidence(
    home: &Path,
    fsv_path: &Path,
    expected_sha: &str,
    replacement: &ActiveVaultIdentity,
) -> CliResult<(FileEvidence, Value)> {
    let resolved = resolve_home_pathbuf(home, fsv_path);
    let (evidence, bytes) = file_evidence(home, &resolved)?;
    if evidence.sha256 != expected_sha {
        return Err(retire_error(
            FSV_MISMATCH_CODE,
            format!(
                "FSV readback {} sha256 {} did not match expected {}",
                resolved.display(),
                evidence.sha256,
                expected_sha
            ),
        ));
    }
    let value: Value = serde_json::from_slice(&bytes).map_err(|error| {
        retire_error(
            FSV_MISMATCH_CODE,
            format!(
                "FSV readback {} is not valid JSON: {error}",
                resolved.display()
            ),
        )
    })?;
    if !value_contains_string(&value, &replacement.vault_id) {
        return Err(retire_error(
            FSV_MISMATCH_CODE,
            format!(
                "FSV readback {} does not reference replacement vault {}",
                resolved.display(),
                replacement.vault_id
            ),
        ));
    }
    Ok((evidence, value))
}

fn identity_from_entry(entry: &Value) -> CliResult<ActiveVaultIdentity> {
    Ok(ActiveVaultIdentity {
        vault_id: required_entry_str(entry, "vault_id")?.to_string(),
        name: required_entry_str(entry, "name")?.to_string(),
        path: required_entry_str(entry, "path")?.to_string(),
        panel_template: entry
            .get("panel_template")
            .and_then(Value::as_str)
            .map(str::to_string),
        original_entry: entry.clone(),
    })
}

fn push_superseded_record(index: &mut Value, record: &VaultSupersessionRecord) -> CliResult {
    if superseded_record(index, &record.superseded_vault_id)?.is_some() {
        return Err(retire_error(
            ALREADY_SUPERSEDED_CODE,
            format!(
                "vault {} already has a superseded_vaults record",
                record.superseded_vault_id
            ),
        ));
    }
    let record_value = serde_json::to_value(record).map_err(|error| {
        CliError::runtime(format!(
            "encode vault supersession record for {}: {error}",
            record.superseded_vault_id
        ))
    })?;
    let object = index.as_object_mut().ok_or_else(|| {
        retire_error(INDEX_CORRUPT_CODE, "vault index root must be a JSON object")
    })?;
    object
        .entry("superseded_vaults")
        .or_insert_with(|| Value::Array(Vec::new()));
    object
        .get_mut("superseded_vaults")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| retire_error(INDEX_CORRUPT_CODE, "superseded_vaults must be an array"))?
        .push(record_value);
    Ok(())
}

fn verify_supersession_readback(index: &Value, record: &VaultSupersessionRecord) -> CliResult {
    if active_position(index, &record.superseded_vault_id)?.is_some() {
        return Err(retire_error(
            READBACK_MISMATCH_CODE,
            format!(
                "vault {} still exists in active vaults after supersession",
                record.superseded_vault_id
            ),
        ));
    }
    if active_position(index, &record.replacement_vault_id)?.is_none() {
        return Err(retire_error(
            READBACK_MISMATCH_CODE,
            format!(
                "replacement vault {} is not active after supersession",
                record.replacement_vault_id
            ),
        ));
    }
    let matches = superseded_array(index)?
        .iter()
        .filter(|entry| superseded_entry_matches(entry, &record.superseded_vault_id))
        .collect::<Vec<_>>();
    if matches.len() != 1 {
        return Err(retire_error(
            READBACK_MISMATCH_CODE,
            format!(
                "vault {} supersession readback expected 1 record, found {}",
                record.superseded_vault_id,
                matches.len()
            ),
        ));
    }
    let decoded: VaultSupersessionRecord =
        serde_json::from_value((*matches[0]).clone()).map_err(|error| {
            retire_error(
                READBACK_MISMATCH_CODE,
                format!(
                    "vault {} supersession record failed to decode during readback: {error}",
                    record.superseded_vault_id
                ),
            )
        })?;
    if decoded.fsv_readback_artifact.sha256 != record.fsv_readback_artifact.sha256
        || decoded.replacement_vault_id != record.replacement_vault_id
        || decoded.reason != record.reason
        || decoded.source_issue != record.source_issue
    {
        return Err(retire_error(
            READBACK_MISMATCH_CODE,
            format!(
                "vault {} supersession record readback did not match written evidence",
                record.superseded_vault_id
            ),
        ));
    }
    Ok(())
}

fn superseded_record<'a>(index: &'a Value, vault: &str) -> CliResult<Option<&'a Value>> {
    Ok(superseded_array(index)?
        .iter()
        .find(|entry| superseded_entry_matches(entry, vault)))
}

fn superseded_vault_count(index: &Value) -> CliResult<usize> {
    Ok(superseded_array(index)?.len())
}

fn superseded_array(index: &Value) -> CliResult<&Vec<Value>> {
    match index.get("superseded_vaults") {
        Some(value) => value
            .as_array()
            .ok_or_else(|| retire_error(INDEX_CORRUPT_CODE, "superseded_vaults must be an array")),
        None => Ok(empty_array()),
    }
}

fn superseded_entry_matches(entry: &Value, vault: &str) -> bool {
    ["superseded_vault_id", "superseded_name", "superseded_path"]
        .iter()
        .any(|field| entry.get(*field).and_then(Value::as_str) == Some(vault))
}

fn resolve_home_path(home: &Path, value: &str) -> PathBuf {
    resolve_home_pathbuf(home, Path::new(value))
}

fn resolve_home_pathbuf(home: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        home.join(path)
    }
}

fn normalize_sha256(value: &str) -> CliResult<String> {
    let normalized = value.trim().to_ascii_lowercase();
    if normalized.len() != 64 || !normalized.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(retire_error(
            FSV_MISMATCH_CODE,
            "FSV readback sha256 must be a 64-character hex digest",
        ));
    }
    Ok(normalized)
}

fn value_contains_string(value: &Value, needle: &str) -> bool {
    match value {
        Value::String(text) => text.contains(needle),
        Value::Array(values) => values
            .iter()
            .any(|value| value_contains_string(value, needle)),
        Value::Object(map) => map
            .iter()
            .any(|(key, value)| key.contains(needle) || value_contains_string(value, needle)),
        _ => false,
    }
}

fn empty_array() -> &'static Vec<Value> {
    static EMPTY: std::sync::OnceLock<Vec<Value>> = std::sync::OnceLock::new();
    EMPTY.get_or_init(Vec::new)
}
