use std::fs;
use std::path::Path;

use serde_json::{Value, json};

use super::support::retire_error;
use super::{
    ALREADY_RETIRED_CODE, INDEX_CORRUPT_CODE, READBACK_MISMATCH_CODE, VaultRetirementRecord,
};
use crate::error::{CliError, CliResult};

pub(super) fn read_index_value(path: &Path) -> CliResult<(Value, Vec<u8>)> {
    if !path.exists() {
        return Ok((json!({"vaults": []}), Vec::new()));
    }
    let bytes = fs::read(path)?;
    let value = serde_json::from_slice(&bytes).map_err(|error| {
        retire_error(
            INDEX_CORRUPT_CODE,
            format!("vault index {} is not valid JSON: {error}", path.display()),
        )
    })?;
    Ok((value, bytes))
}

pub(super) fn active_position(index: &Value, vault: &str) -> CliResult<Option<usize>> {
    Ok(vaults_array(index)?
        .iter()
        .position(|entry| entry_matches(entry, vault)))
}

pub(super) fn retired_record<'a>(index: &'a Value, vault: &str) -> CliResult<Option<&'a Value>> {
    Ok(retired_array(index)?
        .iter()
        .find(|entry| entry_matches(entry, vault)))
}

pub(super) fn push_retired_record(index: &mut Value, record: &VaultRetirementRecord) -> CliResult {
    if retired_record(index, &record.vault_id)?.is_some() {
        return Err(retire_error(
            ALREADY_RETIRED_CODE,
            format!(
                "vault {} already has a retired_vaults record",
                record.vault_id
            ),
        ));
    }
    let record_value = serde_json::to_value(record).map_err(|error| {
        CliError::runtime(format!(
            "encode vault retirement record for {}: {error}",
            record.vault_id
        ))
    })?;
    let object = index.as_object_mut().ok_or_else(|| {
        retire_error(INDEX_CORRUPT_CODE, "vault index root must be a JSON object")
    })?;
    object
        .entry("retired_vaults")
        .or_insert_with(|| Value::Array(Vec::new()));
    object
        .get_mut("retired_vaults")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| retire_error(INDEX_CORRUPT_CODE, "retired_vaults must be an array"))?
        .push(record_value);
    Ok(())
}

pub(super) fn verify_retirement_readback(
    index: &Value,
    record: &VaultRetirementRecord,
) -> CliResult {
    if active_position(index, &record.vault_id)?.is_some() {
        return Err(retire_error(
            READBACK_MISMATCH_CODE,
            format!(
                "vault {} still exists in active vaults after retirement",
                record.vault_id
            ),
        ));
    }
    let matches = retired_array(index)?
        .iter()
        .filter(|entry| entry_matches(entry, &record.vault_id))
        .collect::<Vec<_>>();
    if matches.len() != 1 {
        return Err(retire_error(
            READBACK_MISMATCH_CODE,
            format!(
                "vault {} retirement readback expected 1 record, found {}",
                record.vault_id,
                matches.len()
            ),
        ));
    }
    let decoded: VaultRetirementRecord =
        serde_json::from_value((*matches[0]).clone()).map_err(|error| {
            retire_error(
                READBACK_MISMATCH_CODE,
                format!(
                    "vault {} retirement record failed to decode during readback: {error}",
                    record.vault_id
                ),
            )
        })?;
    if decoded.quarantine_marker.sha256 != record.quarantine_marker.sha256
        || decoded.current_manifest.sha256 != record.current_manifest.sha256
        || decoded.reason != record.reason
    {
        return Err(retire_error(
            READBACK_MISMATCH_CODE,
            format!(
                "vault {} retirement record readback did not match written evidence",
                record.vault_id
            ),
        ));
    }
    Ok(())
}

pub(super) fn active_vault_count(index: &Value) -> CliResult<usize> {
    Ok(vaults_array(index)?.len())
}

pub(super) fn retired_vault_count(index: &Value) -> CliResult<usize> {
    Ok(retired_array(index)?.len())
}

pub(super) fn vaults_array(index: &Value) -> CliResult<&Vec<Value>> {
    index
        .get("vaults")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            retire_error(
                INDEX_CORRUPT_CODE,
                "vault index field vaults must be an array",
            )
        })
}

pub(super) fn vaults_array_mut(index: &mut Value) -> CliResult<&mut Vec<Value>> {
    index
        .get_mut("vaults")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| {
            retire_error(
                INDEX_CORRUPT_CODE,
                "vault index field vaults must be an array",
            )
        })
}

fn retired_array(index: &Value) -> CliResult<&Vec<Value>> {
    match index.get("retired_vaults") {
        Some(value) => value
            .as_array()
            .ok_or_else(|| retire_error(INDEX_CORRUPT_CODE, "retired_vaults must be an array")),
        None => Ok(empty_array()),
    }
}

fn empty_array() -> &'static Vec<Value> {
    static EMPTY: std::sync::OnceLock<Vec<Value>> = std::sync::OnceLock::new();
    EMPTY.get_or_init(Vec::new)
}

fn entry_matches(entry: &Value, vault: &str) -> bool {
    ["vault_id", "name", "path"]
        .iter()
        .any(|field| entry.get(*field).and_then(Value::as_str) == Some(vault))
}

pub(super) fn required_entry_str<'a>(entry: &'a Value, field: &str) -> CliResult<&'a str> {
    entry.get(field).and_then(Value::as_str).ok_or_else(|| {
        retire_error(
            INDEX_CORRUPT_CODE,
            format!("active vault index entry is missing string field {field}"),
        )
    })
}
