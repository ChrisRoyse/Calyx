use std::fs::{self, File};
use std::io::Write;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use calyx_core::{CalyxError, Result};
use serde::Serialize;
use sha2::{Digest, Sha256};

use super::types::{CORRUPT_CODE, MISSING_CODE, REMEDIATION, STALE_CODE};

pub(super) fn write_json_file(path: &Path, value: &impl Serialize) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(value)
        .map_err(|error| corrupt(format!("encode Base page index JSON: {error}")))?;
    write_bytes_file(path, &bytes)
}

pub(super) fn write_bytes_file(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut file = File::create(path).map_err(|error| {
        CalyxError::disk_pressure(format!(
            "create Base page index {}: {error}",
            path.display()
        ))
    })?;
    file.write_all(bytes).map_err(|error| {
        CalyxError::disk_pressure(format!("write Base page index {}: {error}", path.display()))
    })?;
    file.sync_all().map_err(|error| {
        CalyxError::disk_pressure(format!("sync Base page index {}: {error}", path.display()))
    })
}

pub(super) fn remove_path(path: &Path) -> Result<()> {
    if path.is_dir() {
        fs::remove_dir_all(path).map_err(|error| {
            CalyxError::disk_pressure(format!(
                "remove Base page index {}: {error}",
                path.display()
            ))
        })
    } else {
        fs::remove_file(path).map_err(|error| {
            CalyxError::disk_pressure(format!(
                "remove Base page index {}: {error}",
                path.display()
            ))
        })
    }
}

pub(super) fn sync_parent(path: &Path) -> Result<()> {
    crate::fsync::sync_parent(path, "Base page index")
}

pub(super) fn now_ms() -> Result<u128> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .map_err(|error| corrupt(format!("system clock before Unix epoch: {error}")))
}

pub(super) fn relative_path(root: &Path, path: &Path) -> String {
    let relative = path.strip_prefix(root).unwrap_or(path);
    relative.to_string_lossy().replace('\\', "/")
}

pub(super) fn sha256_hex(bytes: impl AsRef<[u8]>) -> String {
    hex_bytes(&Sha256::digest(bytes.as_ref()))
}

pub(super) fn hex_bytes(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(hex_digit(byte >> 4));
        out.push(hex_digit(byte & 0x0f));
    }
    out
}

pub(super) fn decode_hex(raw: &str, label: &str) -> Result<Vec<u8>> {
    if !raw.len().is_multiple_of(2) {
        return Err(corrupt(format!("{label} hex has odd length")));
    }
    let mut bytes = Vec::with_capacity(raw.len() / 2);
    for index in (0..raw.len()).step_by(2) {
        let byte = u8::from_str_radix(&raw[index..index + 2], 16)
            .map_err(|error| corrupt(format!("{label} hex is invalid at {index}: {error}")))?;
        bytes.push(byte);
    }
    Ok(bytes)
}

fn hex_digit(value: u8) -> char {
    match value {
        0..=9 => char::from(b'0' + value),
        10..=15 => char::from(b'a' + value - 10),
        _ => unreachable!("nibble out of range"),
    }
}

pub(super) fn missing(message: impl Into<String>) -> CalyxError {
    index_error(MISSING_CODE, message)
}

pub(super) fn stale(message: impl Into<String>) -> CalyxError {
    index_error(STALE_CODE, message)
}

pub(super) fn corrupt(message: impl Into<String>) -> CalyxError {
    index_error(CORRUPT_CODE, message)
}

fn index_error(code: &'static str, message: impl Into<String>) -> CalyxError {
    CalyxError {
        code,
        message: message.into(),
        remediation: REMEDIATION,
    }
}
