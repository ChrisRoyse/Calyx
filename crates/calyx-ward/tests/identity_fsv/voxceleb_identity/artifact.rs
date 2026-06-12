use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::data::{EmbeddedClip, display};
use super::math::norm;

pub(super) fn embedding_json(clip: &EmbeddedClip) -> Value {
    json!({
        "rel_path": &clip.rel_path,
        "speaker_id": &clip.speaker_id,
        "wav_sha256": &clip.wav_sha256,
        "wav_blake3": &clip.wav_blake3,
        "wav_bytes": clip.wav_bytes,
        "sample_rate": clip.sample_rate,
        "frames": clip.frames,
        "embedding_dim": clip.embedding.len(),
        "embedding_norm": norm(&clip.embedding),
        "embedding_prefix": clip.embedding.iter().take(8).copied().collect::<Vec<_>>(),
        "embedding": &clip.embedding,
    })
}

pub(super) fn clip_ref(clip: &EmbeddedClip) -> Value {
    json!({
        "rel_path": &clip.rel_path,
        "speaker_id": &clip.speaker_id,
        "wav_sha256": &clip.wav_sha256,
    })
}

pub(super) fn write_json(path: &Path, value: &Value) -> PathBuf {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create parent");
    }
    fs::write(path, serde_json::to_vec_pretty(value).unwrap()).expect("write json");
    path.to_path_buf()
}

pub(super) fn file_state(path: &Path) -> Value {
    if !path.exists() {
        return json!({"path": display(path), "exists": false});
    }
    let bytes = fs::read(path).expect("read file state");
    json!({
        "path": display(path),
        "exists": true,
        "len": bytes.len(),
        "blake3": blake3::hash(&bytes).to_string(),
        "hex_prefix": bytes.iter().take(64).map(|byte| format!("{byte:02x}")).collect::<String>(),
    })
}

pub(super) fn write_blake3_manifest(root: &Path) -> PathBuf {
    let mut files = Vec::new();
    collect_files(root, root, &mut files);
    files.sort();
    let mut manifest = String::new();
    for path in files {
        let bytes = fs::read(&path).expect("read manifest input");
        let relative = path
            .strip_prefix(root)
            .unwrap()
            .display()
            .to_string()
            .replace('\\', "/");
        manifest.push_str(&format!("{}  {relative}\n", blake3::hash(&bytes)));
    }
    let path = root.join("BLAKE3SUMS.txt");
    fs::write(&path, manifest).expect("write blake3 manifest");
    path
}

fn collect_files(root: &Path, dir: &Path, files: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).expect("read dir") {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            collect_files(root, &path, files);
        } else if path.strip_prefix(root).unwrap() != Path::new("BLAKE3SUMS.txt") {
            files.push(path);
        }
    }
}
