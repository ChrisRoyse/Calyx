//! WAL segment naming helpers.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub(super) fn segment_path(dir: &Path, index: u64) -> PathBuf {
    dir.join(format!("{index:020}.wal"))
}

pub(super) fn list_segments(dir: &Path) -> io::Result<Vec<(u64, PathBuf)>> {
    if !dir.exists() {
        return Ok(Vec::new());
    }

    let mut segments = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if let Some(index) = segment_index(&path) {
            segments.push((index, path));
        }
    }
    segments.sort_by_key(|(index, _)| *index);
    Ok(segments)
}

fn segment_index(path: &Path) -> Option<u64> {
    if path.extension()? != "wal" {
        return None;
    }
    path.file_stem()?.to_string_lossy().parse().ok()
}
