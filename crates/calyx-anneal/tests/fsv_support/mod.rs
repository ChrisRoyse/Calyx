#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};

use calyx_core::VaultId;
use serde::Serialize;
use serde_json::Value;

pub const DEFAULT_VAULT_ID: &str = "01ARZ3NDEKTSV4RRFFQ69G5FAV";

pub fn vault_id() -> VaultId {
    parse_vault_id(DEFAULT_VAULT_ID)
}

pub fn parse_vault_id(value: &str) -> VaultId {
    value.parse().expect("valid ULID")
}

pub fn write_json<T: Serialize + ?Sized>(path: &Path, value: &T) {
    let bytes = serde_json::to_vec_pretty(value).expect("serialize JSON artifact");
    fs::write(path, bytes).expect("write JSON artifact");
}

pub fn read_json(path: &Path) -> Value {
    serde_json::from_slice(&fs::read(path).unwrap_or_default()).unwrap_or(Value::Null)
}

pub fn write_manifest(root: &Path, paths: &[PathBuf]) {
    let mut lines = String::new();
    for path in paths {
        let bytes = fs::read(path).expect("read manifest artifact");
        let rel = path.strip_prefix(root).unwrap_or(path);
        lines.push_str(&format!(
            "{}  {}\n",
            blake3::hash(&bytes).to_hex(),
            rel.display()
        ));
    }
    fs::write(root.join("BLAKE3SUMS.txt"), lines).expect("write manifest");
}

pub fn reset_dir(dir: &Path) {
    let _ = fs::remove_dir_all(dir);
    fs::create_dir_all(dir).expect("create dir");
}

pub fn physical_files(root: &Path) -> Vec<String> {
    let mut files = Vec::new();
    collect_files(root, root, &mut files);
    files.sort();
    files
}

pub fn collect_files(root: &Path, dir: &Path, files: &mut Vec<String>) {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                collect_files(root, &path, files);
            } else {
                files.push(
                    path.strip_prefix(root)
                        .unwrap_or(&path)
                        .display()
                        .to_string(),
                );
            }
        }
    }
}

pub fn hex(bytes: &[u8]) -> String {
    hex_bytes(bytes)
}

pub fn hex_bytes(bytes: &[u8]) -> String {
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
