use std::fs;
use std::path::Path;

use calyx_aster::cf::ColumnFamily;
use calyx_aster::vault::AsterVault;
use calyx_core::VaultStore;
use calyx_ledger::decode as decode_ledger;
use serde_json::{Value, json};

type RawCfRows = Vec<(Vec<u8>, Vec<u8>)>;

pub(super) fn ledger_payloads(vault: &AsterVault) -> Result<Vec<Value>, String> {
    raw_cf(vault, ColumnFamily::Ledger)?
        .into_iter()
        .map(|(key, value)| {
            let entry = decode_ledger(&value).map_err(|error| error.to_string())?;
            let payload: Value =
                serde_json::from_slice(&entry.payload).map_err(|error| error.to_string())?;
            Ok(json!({
                "key_hex": hex_bytes(&key),
                "payload": payload,
            }))
        })
        .collect()
}

pub(super) fn raw_rows(vault: &AsterVault, cf: ColumnFamily) -> Result<Vec<Value>, String> {
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

fn raw_cf(vault: &AsterVault, cf: ColumnFamily) -> Result<RawCfRows, String> {
    let mut rows = vault
        .scan_cf_at(vault.snapshot(), cf)
        .map_err(|error| error.to_string())?;
    rows.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(rows)
}

pub(super) fn vault_files(root: &Path) -> Result<Vec<Value>, String> {
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

fn collect_files(root: &Path, dir: &Path, files: &mut Vec<Value>) -> Result<(), String> {
    for entry in fs::read_dir(dir).map_err(|error| error.to_string())? {
        let path = entry.map_err(|error| error.to_string())?.path();
        if path.is_dir() {
            collect_files(root, &path, files)?;
        } else {
            let relative = path.strip_prefix(root).map_err(|error| error.to_string())?;
            let bytes = fs::read(&path).map_err(|error| error.to_string())?;
            files.push(json!({
                "path": relative.to_string_lossy().replace('\\', "/"),
                "bytes": bytes.len(),
                "blake3": blake3::hash(&bytes).to_string(),
            }));
        }
    }
    Ok(())
}

pub(super) fn write_json(path: &Path, value: &Value) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    fs::write(
        path,
        serde_json::to_vec_pretty(value).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())
}

fn hex_bytes(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(hex_digit(byte >> 4));
        out.push(hex_digit(byte & 0x0f));
    }
    out
}

fn hex_digit(value: u8) -> char {
    match value {
        0..=9 => char::from(b'0' + value),
        10..=15 => char::from(b'a' + value - 10),
        _ => unreachable!("nibble out of range"),
    }
}
