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

#[derive(Clone, Copy)]
pub enum ManifestPathStyle {
    Display,
    Slash,
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

pub fn write_tree_manifest(root: &Path, style: ManifestPathStyle) {
    let manifest = root.join("BLAKE3SUMS.txt");
    let mut files = Vec::new();
    collect_manifest_files(root, root, &manifest, &mut files);
    files.sort();
    let mut lines = String::new();
    for relative in files {
        let bytes = fs::read(root.join(&relative)).expect("read checksum file");
        let path = match style {
            ManifestPathStyle::Display => relative.display().to_string(),
            ManifestPathStyle::Slash => relative.to_string_lossy().replace('\\', "/"),
        };
        lines.push_str(&format!("{}  {}\n", blake3::hash(&bytes).to_hex(), path));
    }
    fs::write(manifest, lines).expect("write manifest");
}

pub fn write_physical_size_list(path: &Path, root: &Path) {
    let mut lines = Vec::new();
    collect_physical_size_lines(root, root, &mut lines);
    lines.sort();
    fs::write(path, lines.join("\n")).expect("write physical file list");
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

pub fn strict_physical_files(root: &Path) -> Vec<String> {
    let mut files = Vec::new();
    collect_strict_physical_files(root, root, &mut files);
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

fn collect_strict_physical_files(root: &Path, dir: &Path, out: &mut Vec<String>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            collect_strict_physical_files(root, &path, out);
        } else {
            out.push(path.strip_prefix(root).unwrap().display().to_string());
        }
    }
}

fn collect_manifest_files(root: &Path, dir: &Path, manifest: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).expect("read manifest dir") {
        let path = entry.expect("manifest entry").path();
        if path == manifest {
            continue;
        }
        if path.is_dir() {
            collect_manifest_files(root, &path, manifest, out);
        } else {
            out.push(path.strip_prefix(root).expect("relative").to_path_buf());
        }
    }
}

fn collect_physical_size_lines(root: &Path, dir: &Path, lines: &mut Vec<String>) {
    for entry in fs::read_dir(dir).expect("read dir") {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            collect_physical_size_lines(root, &path, lines);
        } else {
            let rel = path.strip_prefix(root).unwrap_or(&path);
            let size = fs::metadata(&path).expect("metadata").len();
            lines.push(format!("{} bytes {}", size, rel.display()));
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
