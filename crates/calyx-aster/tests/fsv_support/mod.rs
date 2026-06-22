#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static TEMP_ROOT_SEQ: AtomicU64 = AtomicU64::new(0);

pub(crate) fn fsv_root(env_key: &str, fallback_prefix: &str) -> (PathBuf, bool) {
    if let Ok(root) = std::env::var(env_key) {
        return (PathBuf::from(root), true);
    }
    (
        std::env::temp_dir().join(format!("{fallback_prefix}-{}", std::process::id())),
        false,
    )
}

pub(crate) fn fsv_root_os(env_key: &str, fallback_prefix: &str) -> PathBuf {
    std::env::var_os(env_key)
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::temp_dir().join(format!("{fallback_prefix}-{}", std::process::id()))
        })
}

pub(crate) fn fsv_root_env_subdir(
    env_key: &str,
    env_subdir: &str,
    fallback_prefix: &str,
) -> (PathBuf, bool) {
    if let Ok(root) = std::env::var(env_key) {
        return (PathBuf::from(root).join(env_subdir), true);
    }
    (
        std::env::temp_dir().join(format!("{fallback_prefix}-{}", std::process::id())),
        false,
    )
}

pub(crate) fn named_fsv_root(env_key: &str, name: &str) -> (PathBuf, bool) {
    if let Ok(root) = std::env::var(env_key) {
        return (PathBuf::from(root), true);
    }
    (named_temp_root(name), false)
}

pub(crate) fn named_fsv_root_os(env_key: &str, name: &str) -> (PathBuf, bool) {
    if let Some(root) = std::env::var_os(env_key) {
        return (PathBuf::from(root), true);
    }
    (named_temp_root(name), false)
}

fn named_temp_root(name: &str) -> PathBuf {
    temp_root("calyx-aster", name)
}

pub(crate) fn temp_root(prefix: &str, name: &str) -> PathBuf {
    let id = TEMP_ROOT_SEQ.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("{prefix}-{name}-{}-{id}", std::process::id()))
}

pub(crate) fn prepared_temp_root(prefix: &str, name: &str) -> PathBuf {
    let dir = temp_root(prefix, name);
    reset_dir(&dir);
    dir
}

pub(crate) fn env_or_temp_root(
    env_key: &str,
    fallback_prefix: &str,
    fallback_name: &str,
) -> PathBuf {
    std::env::var_os(env_key)
        .map(PathBuf::from)
        .unwrap_or_else(|| temp_root(fallback_prefix, fallback_name))
}

pub(crate) fn env_or_prepared_temp_root(
    env_key: &str,
    fallback_prefix: &str,
    fallback_name: &str,
) -> PathBuf {
    std::env::var_os(env_key)
        .map(PathBuf::from)
        .unwrap_or_else(|| prepared_temp_root(fallback_prefix, fallback_name))
}

pub(crate) fn reset_dir(dir: &Path) {
    let _ = fs::remove_dir_all(dir);
    fs::create_dir_all(dir).expect("create fsv root");
}

pub(crate) fn write_json(path: &Path, value: &serde_json::Value) {
    fs::write(path, serde_json::to_vec_pretty(value).expect("json")).expect("write json");
}

pub(crate) fn collect_physical_file_states(root: &Path, files: &mut Vec<serde_json::Value>) {
    for entry in fs::read_dir(root).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        if path.is_dir() {
            collect_physical_file_states(&path, files);
        } else {
            files.push(serde_json::json!({
                "path": path.display().to_string(),
                "bytes": entry.metadata().unwrap().len(),
            }));
        }
    }
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
            blake3_hex(&bytes),
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
            files.push(
                path.strip_prefix(root)
                    .expect("relative path")
                    .to_path_buf(),
            );
        }
    }
}

pub(crate) fn blake3_hex(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}
