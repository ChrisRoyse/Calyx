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

#[cfg(windows)]
fn sync_parent_dir(parent: &Path, label: &str) -> CliResult {
    use std::fs::OpenOptions;
    use std::os::windows::fs::OpenOptionsExt;

    use windows_sys::Win32::Storage::FileSystem::FILE_FLAG_BACKUP_SEMANTICS;

    let dir = OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
        .open(parent)
        .map_err(|error| {
            CliError::io(format!(
                "open {label} parent directory {} for Windows sync failed: {error}",
                parent.display()
            ))
        })?;
    dir.sync_all().map_err(|error| {
        CliError::io(format!(
            "sync {label} parent directory {} on Windows failed: {error}",
            parent.display()
        ))
    })
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;
    use std::time::{SystemTime, UNIX_EPOCH};

    use serde_json::json;

    use super::*;

    #[test]
    fn atomic_json_write_publishes_and_removes_temp_file() {
        let root = std::env::temp_dir().join(format!(
            "calyx-durable-write-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock after epoch")
                .as_nanos()
        ));
        fs::create_dir_all(&root).expect("create temp durable-write root");
        let path = root.join("nested").join("matrix.json");
        let value = json!({
            "schema": "calyx-durable-write-test-v1",
            "source_of_truth": "physical file readback after atomic publish"
        });

        write_json_value_atomic(&path, &value, "durable write test").expect("atomic write");

        let bytes = fs::read(&path).expect("read published json");
        let decoded: Value = serde_json::from_slice(&bytes).expect("decode published json");
        assert_eq!(decoded, value);
        assert!(
            !temp_path(&path).expect("temp path").exists(),
            "temporary file must not remain after publish"
        );

        fs::remove_dir_all(&root).expect("cleanup temp durable-write root");
    }

    #[test]
    fn atomic_write_overwrites_existing_file_and_reads_back_new_bytes() {
        let root = temp_root("overwrite");
        let path = root.join("progress.json");
        fs::create_dir_all(&root).expect("create temp durable-write root");
        write_bytes_atomic(&path, b"before\n", "overwrite test").expect("initial write");

        write_bytes_atomic(&path, b"after\n", "overwrite test").expect("overwrite");

        assert_eq!(fs::read(&path).expect("read overwritten file"), b"after\n");
        assert!(
            !temp_path(&path).expect("temp path").exists(),
            "temporary file must not remain after overwrite"
        );
        fs::remove_dir_all(&root).expect("cleanup temp durable-write root");
    }

    #[test]
    fn atomic_write_fails_when_parent_path_is_file() {
        let root = temp_root("parent-file");
        fs::create_dir_all(&root).expect("create temp durable-write root");
        let blocked_parent = root.join("blocked");
        fs::write(&blocked_parent, b"not a directory").expect("write blocking file");
        let path = blocked_parent.join("progress.json");

        let error = write_bytes_atomic(&path, b"unpublished\n", "parent file test")
            .expect_err("parent file must fail closed");

        assert!(
            error
                .to_string()
                .contains("create parent file test parent directory"),
            "error should name the failing parent creation: {error}"
        );
        assert!(
            !path.exists(),
            "child file must not be published when parent is a file"
        );
        assert_eq!(
            fs::read(&blocked_parent).expect("read blocking file"),
            b"not a directory"
        );
        fs::remove_dir_all(&root).expect("cleanup temp durable-write root");
    }

    #[test]
    fn atomic_write_rejects_empty_path_without_publishing() {
        let error = write_bytes_atomic(Path::new(""), b"unpublished\n", "empty path test")
            .expect_err("empty path must fail closed");

        assert!(
            error
                .to_string()
                .contains("empty path test path  has no parent"),
            "error should name the missing parent: {error}"
        );
    }

    fn temp_root(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "calyx-durable-write-{label}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock after epoch")
                .as_nanos()
        ))
    }
}
