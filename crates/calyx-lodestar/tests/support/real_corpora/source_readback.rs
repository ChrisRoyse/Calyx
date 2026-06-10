use std::fs;
use std::path::{Path, PathBuf};

use calyx_core::content_address;
use serde::Serialize;

#[derive(Serialize)]
pub(super) struct SourceReadback {
    pub path: String,
    pub bytes: u64,
    pub file_count: usize,
    pub content_hash: String,
}

struct SourceBytes {
    bytes: u64,
    file_count: usize,
    content_hash: String,
}

pub(super) fn source_readbacks(paths: &[PathBuf]) -> Vec<SourceReadback> {
    paths
        .iter()
        .map(|path| {
            let bytes = read_source_bytes(path);
            SourceReadback {
                path: path.display().to_string(),
                bytes: bytes.bytes,
                file_count: bytes.file_count,
                content_hash: bytes.content_hash,
            }
        })
        .collect()
}

fn read_source_bytes(path: &Path) -> SourceBytes {
    if path.is_dir() {
        let mut files = Vec::new();
        collect_files(path, &mut files);
        files.sort();
        let mut parts = Vec::new();
        let mut total = 0_u64;
        for file in &files {
            let body = fs::read(file).expect("read source file");
            total += body.len() as u64;
            parts.push(
                file.strip_prefix(path)
                    .unwrap_or(file)
                    .display()
                    .to_string()
                    .into_bytes(),
            );
            parts.push(body);
        }
        SourceBytes {
            bytes: total,
            file_count: files.len(),
            content_hash: hex(&content_address(parts)),
        }
    } else {
        let body = fs::read(path).expect("read source");
        SourceBytes {
            bytes: body.len() as u64,
            file_count: 1,
            content_hash: hex(&content_address([body])),
        }
    }
}

fn collect_files(root: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(root).expect("read source dir") {
        let path = entry.expect("source entry").path();
        if path.is_dir() {
            collect_files(&path, out);
        } else {
            out.push(path);
        }
    }
}

fn hex(bytes: &[u8; 16]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
