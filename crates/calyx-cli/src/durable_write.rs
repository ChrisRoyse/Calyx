use std::ffi::OsString;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::error::{CliError, CliResult};

pub(crate) fn write_json_value_atomic(path: &Path, value: &Value, label: &str) -> CliResult {
    let mut bytes = serde_json::to_vec_pretty(value)?;
    bytes.push(10);
    write_bytes_atomic(path, &bytes, label)
}

pub(crate) fn write_bytes_atomic(path: &Path, bytes: &[u8], label: &str) -> CliResult {
    let parent = path
        .parent()
        .ok_or_else(|| CliError::io(format!("{label} path {} has no parent", path.display())))?;
    fs::create_dir_all(parent).map_err(|error| {
        CliError::io(format!(
            "create {label} parent directory {} failed: {error}",
            parent.display()
        ))
    })?;
    let tmp = temp_path(path)?;
    let mut file = File::create(&tmp).map_err(|error| {
        CliError::io(format!(
            "create temporary {label} {} failed: {error}",
            tmp.display()
        ))
    })?;
    file.write_all(bytes).map_err(|error| {
        CliError::io(format!(
            "write temporary {label} {} failed: {error}",
            tmp.display()
        ))
    })?;
    file.sync_all().map_err(|error| {
        CliError::io(format!(
            "sync temporary {label} {} failed: {error}",
            tmp.display()
        ))
    })?;
    drop(file);
    fs::rename(&tmp, path).map_err(|error| {
        CliError::io(format!(
            "publish {label} {} -> {} failed: {error}",
            tmp.display(),
            path.display()
        ))
    })?;
    sync_parent_dir(parent, label)
}

fn temp_path(path: &Path) -> CliResult<PathBuf> {
    let filename = path.file_name().ok_or_else(|| {
        CliError::io(format!(
            "atomic write path {} has no filename",
            path.display()
        ))
    })?;
    let mut tmp_name = OsString::from(".");
    tmp_name.push(filename);
    tmp_name.push(format!(".{}.tmp", std::process::id()));
    Ok(path.with_file_name(tmp_name))
}

#[cfg(unix)]
fn sync_parent_dir(parent: &Path, label: &str) -> CliResult {
    let dir = File::open(parent).map_err(|error| {
        CliError::io(format!(
            "open {label} parent directory {} for sync failed: {error}",
            parent.display()
        ))
    })?;
    dir.sync_all().map_err(|error| {
        CliError::io(format!(
            "sync {label} parent directory {} failed: {error}",
            parent.display()
        ))
    })
}
