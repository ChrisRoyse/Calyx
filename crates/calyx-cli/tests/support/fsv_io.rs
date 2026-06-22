use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

pub fn fsv_root(env_key: &str, fallback: &str) -> PathBuf {
    std::env::var(env_key)
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir().join(fallback))
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

pub fn write_json(path: &Path, value: &Value) {
    fs::write(
        path,
        serde_json::to_vec_pretty(value).expect("serialize json"),
    )
    .expect("write json");
}

pub fn write_blake3_sums(root: &Path) {
    let mut lines = Vec::new();
    collect_hashes(root, root, &mut lines);
    lines.sort();
    fs::write(root.join("BLAKE3SUMS.txt"), lines.concat()).expect("write manifest");
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
