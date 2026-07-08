use std::path::Path;

use calyx_core::{CalyxError, Result};

pub(crate) fn sync_parent(path: &Path, label: &str) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| CalyxError::disk_pressure(format!("{label} path has no parent")))?;
    sync_dir(parent, label)
}

#[cfg(unix)]
pub(crate) fn sync_dir(dir: &Path, label: &str) -> Result<()> {
    use std::fs::File;

    ensure_directory(dir, label)?;
    File::open(dir)
        .and_then(|handle| handle.sync_all())
        .map_err(|error| {
            CalyxError::disk_pressure(format!(
                "sync {label} parent directory {}: {error}",
                dir.display()
            ))
        })
}

#[cfg(windows)]
pub(crate) fn sync_dir(dir: &Path, label: &str) -> Result<()> {
    use std::fs::OpenOptions;
    use std::os::windows::fs::OpenOptionsExt;

    use windows_sys::Win32::Storage::FileSystem::FILE_FLAG_BACKUP_SEMANTICS;

    ensure_directory(dir, label)?;
    OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
        .open(dir)
        .and_then(|handle| handle.sync_all())
        .map_err(|error| {
            CalyxError::disk_pressure(format!(
                "sync {label} parent directory {} on Windows: {error}",
                dir.display()
            ))
        })
}

#[cfg(not(any(unix, windows)))]
pub(crate) fn sync_dir(dir: &Path, label: &str) -> Result<()> {
    ensure_directory(dir, label)?;
    Err(CalyxError::disk_pressure(format!(
        "sync {label} parent directory {}: unsupported platform",
        dir.display()
    )))
}

fn ensure_directory(dir: &Path, label: &str) -> Result<()> {
    if dir.is_dir() {
        return Ok(());
    }
    Err(CalyxError::disk_pressure(format!(
        "sync {label} parent directory {}: not a directory",
        dir.display()
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn sync_parent_succeeds_for_real_directory() {
        let dir = test_dir("real-dir");
        let path = dir.join("published.bin");
        fs::write(&path, b"durable-parent-sync").expect("write file");

        sync_parent(&path, "fsync unit test").expect("sync parent");

        fs::remove_dir_all(dir).expect("cleanup");
    }

    #[test]
    fn sync_dir_rejects_file_path() {
        let dir = test_dir("file-path");
        let path = dir.join("not-a-directory");
        fs::write(&path, b"not a directory").expect("write file");

        let error = sync_dir(&path, "fsync unit test").expect_err("file is not a directory");

        assert_eq!(error.code, "CALYX_DISK_PRESSURE");
        fs::remove_dir_all(dir).expect("cleanup");
    }

    fn test_dir(name: &str) -> std::path::PathBuf {
        let id = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "calyx-aster-fsync-{name}-{}-{id}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create test dir");
        dir
    }
}
