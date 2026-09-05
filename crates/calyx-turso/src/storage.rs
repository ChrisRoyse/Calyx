use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};

use crate::error;

pub(crate) fn prepare(path: &Path) -> calyx_core::Result<()> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let parent_metadata = fs::symlink_metadata(parent).map_err(|source| {
        error::permissions(format!(
            "inspect database parent {}: {source}",
            parent.display()
        ))
    })?;
    if parent_metadata.file_type().is_symlink() || !parent_metadata.is_dir() {
        return Err(error::permissions(format!(
            "database parent {} is not a real directory",
            parent.display()
        )));
    }

    let mut created_any = false;
    for storage_path in storage_paths(path) {
        created_any |= prepare_file(&storage_path)?;
    }
    if created_any {
        sync_parent(parent)?;
    }
    Ok(())
}

pub(crate) fn verify(path: &Path) -> calyx_core::Result<()> {
    for storage_path in storage_paths(path) {
        verify_file(&storage_path)?;
    }
    Ok(())
}

fn storage_paths(path: &Path) -> [PathBuf; 3] {
    let mut wal = OsString::from(path.as_os_str());
    wal.push("-wal");
    [
        path.to_path_buf(),
        path.with_extension("db-log"),
        wal.into(),
    ]
}

fn prepare_file(path: &Path) -> calyx_core::Result<bool> {
    match create_private_file(path) {
        Ok(file) => {
            file.sync_all().map_err(|source| {
                error::permissions(format!(
                    "sync new storage file {}: {source}",
                    path.display()
                ))
            })?;
            verify_open_file(path, &file)?;
            Ok(true)
        }
        Err(source) if source.kind() == std::io::ErrorKind::AlreadyExists => {
            verify_file(path)?;
            Ok(false)
        }
        Err(source) => Err(error::permissions(format!(
            "create storage file {}: {source}",
            path.display()
        ))),
    }
}

#[cfg(unix)]
fn create_private_file(path: &Path) -> std::io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt;

    OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(nix::libc::O_CLOEXEC | nix::libc::O_NOFOLLOW)
        .open(path)
}

#[cfg(not(unix))]
fn create_private_file(path: &Path) -> std::io::Result<File> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(path)
}

fn verify_file(path: &Path) -> calyx_core::Result<()> {
    let metadata = fs::symlink_metadata(path).map_err(|source| {
        error::permissions(format!("inspect storage file {}: {source}", path.display()))
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(error::permissions(format!(
            "{} is not a regular non-symlink storage file",
            path.display()
        )));
    }
    let file = open_verified_file(path)?;
    verify_open_file(path, &file)
}

#[cfg(unix)]
fn open_verified_file(path: &Path) -> calyx_core::Result<File> {
    use std::os::unix::fs::OpenOptionsExt;

    OpenOptions::new()
        .read(true)
        .custom_flags(nix::libc::O_CLOEXEC | nix::libc::O_NOFOLLOW)
        .open(path)
        .map_err(|source| {
            error::permissions(format!("open storage file {}: {source}", path.display()))
        })
}

#[cfg(not(unix))]
fn open_verified_file(path: &Path) -> calyx_core::Result<File> {
    OpenOptions::new().read(true).open(path).map_err(|source| {
        error::permissions(format!("open storage file {}: {source}", path.display()))
    })
}

#[cfg(unix)]
fn verify_open_file(path: &Path, file: &File) -> calyx_core::Result<()> {
    use std::os::unix::fs::MetadataExt;

    let metadata = file.metadata().map_err(|source| {
        error::permissions(format!(
            "read storage metadata {}: {source}",
            path.display()
        ))
    })?;
    let effective_uid = unsafe { nix::libc::geteuid() };
    if !metadata.is_file()
        || metadata.nlink() != 1
        || metadata.uid() != effective_uid
        || metadata.mode() & 0o777 != 0o600
    {
        return Err(error::permissions(format!(
            "{} must be a regular, non-hardlinked, uid-{effective_uid} mode-0600 file; observed uid={}, links={}, mode={:04o}",
            path.display(),
            metadata.uid(),
            metadata.nlink(),
            metadata.mode() & 0o777
        )));
    }
    Ok(())
}

#[cfg(not(unix))]
fn verify_open_file(path: &Path, file: &File) -> calyx_core::Result<()> {
    let metadata = file.metadata().map_err(|source| {
        error::permissions(format!(
            "read storage metadata {}: {source}",
            path.display()
        ))
    })?;
    if !metadata.is_file() {
        return Err(error::permissions(format!(
            "{} is not a regular storage file",
            path.display()
        )));
    }
    Ok(())
}

#[cfg(unix)]
fn sync_parent(parent: &Path) -> calyx_core::Result<()> {
    File::open(parent)
        .and_then(|file| file.sync_all())
        .map_err(|source| {
            error::permissions(format!(
                "sync database parent {}: {source}",
                parent.display()
            ))
        })
}

#[cfg(not(unix))]
fn sync_parent(_parent: &Path) -> calyx_core::Result<()> {
    Ok(())
}
