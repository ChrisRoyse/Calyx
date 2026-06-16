use super::CALYX_IO_ERROR;
use calyx_core::{CalyxError, Result, Ts};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const IO_REMEDIATION: &str =
    "inspect janitor filesystem source of truth; preserve files until cleanup is safe";

#[derive(Clone, Copy)]
pub(super) enum CleanupKind {
    Log,
    Temp,
}

pub(super) fn collect_files(root: &Path) -> Result<Vec<PathBuf>> {
    if !root.exists() {
        return Ok(Vec::new());
    }
    let mut files = Vec::new();
    let mut dirs = vec![root.to_path_buf()];
    while let Some(dir) = dirs.pop() {
        for entry in fs::read_dir(&dir)
            .map_err(|error| io_error(format!("read {}: {error}", dir.display())))?
        {
            let entry = entry
                .map_err(|error| io_error(format!("read {} entry: {error}", dir.display())))?;
            let path = entry.path();
            let meta = fs::symlink_metadata(&path)
                .map_err(|error| io_error(format!("stat {}: {error}", path.display())))?;
            if meta.file_type().is_dir() && !meta.file_type().is_symlink() {
                dirs.push(path);
            } else {
                files.push(path);
            }
        }
    }
    Ok(files)
}

pub(super) fn immediate_dirs(root: &Path) -> Result<Vec<PathBuf>> {
    let mut dirs = Vec::new();
    for entry in
        fs::read_dir(root).map_err(|error| io_error(format!("read {}: {error}", root.display())))?
    {
        let path = entry
            .map_err(|error| io_error(format!("read {} entry: {error}", root.display())))?
            .path();
        if path.is_dir() {
            dirs.push(path);
        }
    }
    Ok(dirs)
}

pub(super) fn temp_dirs(home: &Path) -> Result<Vec<PathBuf>> {
    let mut dirs = Vec::new();
    for candidate in [home.join(".tmp"), home.join("data").join(".tmp")] {
        if candidate.is_dir() {
            dirs.push(candidate);
        }
    }
    for dir in immediate_dirs(home)? {
        let tmp = dir.join(".tmp");
        if tmp.is_dir() {
            dirs.push(tmp);
        }
        for child in immediate_dirs(&dir).unwrap_or_default() {
            let nested = child.join(".tmp");
            if nested.is_dir() {
                dirs.push(nested);
            }
        }
    }
    dirs.sort();
    dirs.dedup();
    Ok(dirs)
}

pub(super) fn ensure_inside_dataset(dataset_root: &Path, path: &Path) -> Result<()> {
    let root = dataset_root
        .canonicalize()
        .map_err(|error| io_error(format!("canonicalize {}: {error}", dataset_root.display())))?;
    let actual = path
        .canonicalize()
        .map_err(|error| io_error(format!("canonicalize {}: {error}", path.display())))?;
    if !actual.starts_with(&root) {
        return Err(io_error(format!(
            "temp file {} escapes dataset {}",
            path.display(),
            dataset_root.display()
        )));
    }
    Ok(())
}

pub(super) fn dir_size(path: &Path) -> Result<u64> {
    collect_files(path)?
        .into_iter()
        .map(|file| file_len(&file))
        .sum()
}

pub(super) fn file_len(path: &Path) -> Result<u64> {
    fs::metadata(path)
        .map(|meta| meta.len())
        .map_err(|error| io_error(format!("stat {}: {error}", path.display())))
}

pub(super) fn age_ms(path: &Path, now: Ts) -> Result<u64> {
    Ok(now.saturating_sub(modified_ms(path)?))
}

pub(super) fn modified_ms(path: &Path) -> Result<u64> {
    let modified = fs::metadata(path)
        .map_err(|error| io_error(format!("stat {}: {error}", path.display())))?
        .modified()
        .map_err(|error| io_error(format!("modified {}: {error}", path.display())))?;
    system_time_ms(modified)
}

fn system_time_ms(time: SystemTime) -> Result<u64> {
    time.duration_since(UNIX_EPOCH)
        .map_err(|error| io_error(format!("mtime before epoch: {error}")))?
        .as_millis()
        .try_into()
        .map_err(|_| io_error("mtime exceeds u64 milliseconds"))
}

pub(super) fn duration_ms(duration: Duration) -> u64 {
    duration.as_millis().try_into().unwrap_or(u64::MAX)
}

pub(super) fn is_zst(path: &Path) -> bool {
    path.extension().is_some_and(|ext| ext == "zst")
}

pub(super) fn zst_path(path: &Path) -> Result<PathBuf> {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            io_error(format!(
                "log path has no UTF-8 file name: {}",
                path.display()
            ))
        })?;
    Ok(path.with_file_name(format!("{name}.zst")))
}

pub(super) fn starts_with_canonical(child: &Path, parent: &Path) -> bool {
    parent
        .canonicalize()
        .map(|parent| child.starts_with(parent))
        .unwrap_or(false)
}

pub(super) fn hash_path(path: &Path) -> String {
    blake3::hash(path.to_string_lossy().as_bytes())
        .to_hex()
        .to_string()
}

pub(super) fn io_error(message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: CALYX_IO_ERROR,
        message: message.into(),
        remediation: IO_REMEDIATION,
    }
}
