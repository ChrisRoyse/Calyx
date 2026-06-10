use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

pub(crate) fn write_json(path: &Path, value: &Value) {
    fs::write(path, serde_json::to_vec_pretty(value).expect("json")).expect("write json");
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

pub(crate) fn list_files(dir: &Path) -> Vec<String> {
    let mut files = Vec::new();
    if dir.exists() {
        collect_list(dir, dir, &mut files);
    }
    files.sort();
    files
}

fn collect_list(root: &Path, dir: &Path, files: &mut Vec<String>) {
    for entry in fs::read_dir(dir).expect("read dir") {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            collect_list(root, &path, files);
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

pub(crate) fn fsv_root() -> (PathBuf, bool) {
    if let Ok(root) = std::env::var("CALYX_DEDUP_INVARIANTS_FSV_ROOT") {
        return (PathBuf::from(root), true);
    }
    (
        std::env::temp_dir().join(format!("calyx-dedup-invariants-fsv-{}", std::process::id())),
        false,
    )
}

pub(crate) fn reset_dir(dir: &Path) {
    let _ = fs::remove_dir_all(dir);
    fs::create_dir_all(dir).expect("create fsv root");
}
