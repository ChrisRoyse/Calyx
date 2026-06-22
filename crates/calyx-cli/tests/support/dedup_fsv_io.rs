#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

pub(crate) fn fsv_root(env_key: &str, fallback_prefix: &str) -> (PathBuf, bool) {
    if let Ok(root) = std::env::var(env_key) {
        return (PathBuf::from(root), true);
    }
    (
        std::env::temp_dir().join(format!("{fallback_prefix}-{}", std::process::id())),
        false,
    )
}

pub(crate) fn reset_dir(dir: &Path) {
    let _ = fs::remove_dir_all(dir);
    fs::create_dir_all(dir).expect("create fsv root");
}

pub(crate) fn write_json(path: &Path, value: &Value) {
    fs::write(path, serde_json::to_vec_pretty(value).expect("json")).expect("write json");
}

pub(crate) fn write_text(path: &Path, value: impl AsRef<str>) {
    fs::write(path, value.as_ref()).expect("write text");
}

pub(crate) fn write_blake3_sums(root: &Path) {
    let mut files = Vec::new();
    collect_files(root, root, &mut files);
    files.sort();
    let mut lines = String::new();
    for relative in files {
        if relative == Path::new("BLAKE3SUMS.txt") {
            continue;
        }
        let bytes = fs::read(root.join(&relative)).expect("read checksum file");
        lines.push_str(&format!(
            "{}  {}\n",
            blake3::hash(&bytes).to_hex(),
            relative.to_string_lossy().replace('\\', "/")
        ));
    }
    fs::write(root.join("BLAKE3SUMS.txt"), lines).expect("write checksum manifest");
}

fn collect_files(root: &Path, dir: &Path, files: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).expect("read dir") {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            collect_files(root, &path, files);
        } else {
            files.push(path.strip_prefix(root).unwrap().to_path_buf());
        }
    }
}

pub(crate) fn list_dir_files(dir: &Path) -> Vec<String> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files = entries
        .map(|entry| {
            entry
                .expect("entry")
                .path()
                .strip_prefix(dir)
                .expect("relative")
                .to_string_lossy()
                .replace('\\', "/")
        })
        .collect::<Vec<_>>();
    files.sort();
    files
}

pub(crate) fn list_tree_files(dir: &Path) -> Vec<String> {
    let mut files = Vec::new();
    if dir.exists() {
        collect_tree_files(dir, dir, &mut files);
    }
    files.sort();
    files
}

fn collect_tree_files(root: &Path, dir: &Path, files: &mut Vec<String>) {
    for entry in fs::read_dir(dir).expect("read dir") {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            collect_tree_files(root, &path, files);
        } else {
            files.push(
                path.strip_prefix(root)
                    .expect("relative")
                    .to_string_lossy()
                    .replace('\\', "/"),
            );
        }
    }
}
