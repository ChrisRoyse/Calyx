use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::Value;

static TEMP_ROOT_SEQ: AtomicU64 = AtomicU64::new(0);

pub fn fsv_root(env_key: &str, fallback: &str) -> PathBuf {
    std::env::var(env_key)
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir().join(fallback))
}

pub fn preserved_fsv_root(env_key: &str, fallback_prefix: &str) -> (PathBuf, bool) {
    if let Ok(root) = std::env::var(env_key) {
        return (PathBuf::from(root), true);
    }
    (
        std::env::temp_dir().join(format!("{fallback_prefix}-{}", std::process::id())),
        false,
    )
}

pub fn case_fsv_root(env_key: &str, fallback_prefix: &str, name: &str) -> (PathBuf, bool) {
    if let Ok(root) = std::env::var(env_key) {
        return (PathBuf::from(root).join(name), true);
    }
    let seq = TEMP_ROOT_SEQ.fetch_add(1, Ordering::SeqCst);
    (
        std::env::temp_dir().join(format!(
            "{fallback_prefix}-{name}-{}-{seq}",
            std::process::id()
        )),
        false,
    )
}

pub fn named_temp_root(prefix: &str, name: &str) -> PathBuf {
    let id = TEMP_ROOT_SEQ.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("{prefix}-{name}-{}-{id}", std::process::id()))
}

pub fn numbered_temp_root(prefix: &str, name: &str) -> PathBuf {
    let id = TEMP_ROOT_SEQ.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("{prefix}-{name}-{id}"))
}

pub fn reset_dir(dir: &Path) {
    let _ = fs::remove_dir_all(dir);
    fs::create_dir_all(dir).unwrap();
}

pub fn list_files(dir: &Path) -> Vec<String> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files = entries
        .map(|entry| entry.unwrap().file_name().to_string_lossy().to_string())
        .collect::<Vec<_>>();
    files.sort();
    files
}

pub fn list_tree_files(dir: &Path) -> Vec<String> {
    let mut files = Vec::new();
    if dir.exists() {
        collect_tree_files(dir, dir, &mut files);
    }
    files.sort();
    files
}

pub fn write_json(path: &Path, value: &Value) {
    fs::write(
        path,
        serde_json::to_vec_pretty(value).expect("serialize json"),
    )
    .expect("write json");
}

pub fn write_manifest_asset(vault: &Path, logical_path: &str, bytes: &[u8]) {
    let path = vault.join(logical_path);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, bytes).unwrap();
}

pub fn write_text(path: &Path, value: impl AsRef<str>) {
    fs::write(path, value.as_ref()).expect("write text");
}

pub fn write_blake3_sums(root: &Path) {
    let mut lines = Vec::new();
    collect_hashes(root, root, &mut lines);
    lines.sort();
    fs::write(root.join("BLAKE3SUMS.txt"), lines.concat()).expect("write manifest");
}

pub fn write_blake3_sums_by_path(root: &Path) {
    let mut files = Vec::new();
    collect_paths(root, root, &mut files);
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

fn collect_hashes(root: &Path, path: &Path, lines: &mut Vec<String>) {
    for entry in fs::read_dir(path).expect("read fsv dir") {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            collect_hashes(root, &path, lines);
        } else if path.file_name().unwrap() != "BLAKE3SUMS.txt" {
            let bytes = fs::read(&path).expect("read fsv file");
            let rel = path
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            lines.push(format!("{}  {}\n", blake3::hash(&bytes).to_hex(), rel));
        }
    }
}

fn collect_paths(root: &Path, dir: &Path, files: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).expect("read dir") {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            collect_paths(root, &path, files);
        } else {
            files.push(
                path.strip_prefix(root)
                    .expect("relative path")
                    .to_path_buf(),
            );
        }
    }
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
