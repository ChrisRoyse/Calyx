use std::fs;
use std::path::Path;

use calyx_aster::cf::ColumnFamily;
use calyx_aster::vault::AsterVault;
use calyx_core::VaultStore;
use calyx_ledger::decode as decode_ledger;
use serde_json::{Value, json};

use crate::cf_read::hex_bytes;
use crate::error::{CliError, CliResult};

type RawCfRows = Vec<(Vec<u8>, Vec<u8>)>;

pub(super) fn ledger_payloads(vault: &AsterVault) -> CliResult<Vec<Value>> {
    raw_cf(vault, ColumnFamily::Ledger)?
        .into_iter()
        .map(|(key, value)| {
            let entry = decode_ledger(&value)?;
            let payload: Value = serde_json::from_slice(&entry.payload)
                .map_err(|error| CliError::runtime(format!("parse ledger payload: {error}")))?;
            Ok(json!({
                "key_hex": hex_bytes(&key),
                "payload": payload,
            }))
        })
        .collect()
}

pub(super) fn raw_rows(vault: &AsterVault, cf: ColumnFamily) -> CliResult<Vec<Value>> {
    raw_cf(vault, cf).map(|rows| {
        rows.into_iter()
            .map(|(key, value)| {
                json!({
                    "key_hex": hex_bytes(&key),
                    "value_len": value.len(),
                    "value_hex": hex_bytes(&value),
                })
            })
            .collect()
    })
}

fn raw_cf(vault: &AsterVault, cf: ColumnFamily) -> CliResult<RawCfRows> {
    let mut rows = vault.scan_cf_at(vault.snapshot(), cf)?;
    rows.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(rows)
}

pub(super) fn vault_files(root: &Path) -> CliResult<Vec<Value>> {
    let mut files = Vec::new();
    collect_files(root, root, &mut files)?;
    files.sort_by(|left, right| {
        left["path"]
            .as_str()
            .unwrap_or("")
            .cmp(right["path"].as_str().unwrap_or(""))
    });
    Ok(files)
}

fn collect_files(root: &Path, dir: &Path, files: &mut Vec<Value>) -> CliResult<()> {
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            collect_files(root, &path, files)?;
        } else {
            let relative = path.strip_prefix(root).map_err(|error| {
                CliError::runtime(format!(
                    "strip prefix {} from {}: {error}",
                    root.display(),
                    path.display()
                ))
            })?;
            let bytes = fs::read(&path)?;
            files.push(json!({
                "path": relative.to_string_lossy().replace('\\', "/"),
                "bytes": bytes.len(),
                "blake3": blake3::hash(&bytes).to_string(),
            }));
        }
    }
    Ok(())
}

pub(super) fn write_json(path: &Path, value: &Value) -> CliResult<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let bytes = serde_json::to_vec_pretty(value)
        .map_err(|error| CliError::runtime(format!("serialize {}: {error}", path.display())))?;
    Ok(fs::write(path, bytes)?)
}
