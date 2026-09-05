use std::path::Path;

use super::super::JournalError;

pub(in crate::journal) fn validate_journal_path(path: &Path) -> Result<(), JournalError> {
    if !path.is_absolute() || path.file_name().is_none() {
        return Err(JournalError::InvalidPath(format!(
            "{} must be an absolute file path",
            path.display()
        )));
    }
    let parent = path
        .parent()
        .ok_or_else(|| JournalError::InvalidPath("journal has no parent".into()))?;
    validate_journal_parent(parent)
}

pub(in crate::journal) fn prepare_journal_file(path: &Path) -> Result<bool, JournalError> {
    let result = prepare_journal_file_platform(path);
    match result {
        Ok(file) => {
            file.sync_all().map_err(|source| JournalError::FileSync {
                path: path.into(),
                source,
            })?;
            Ok(true)
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            validate_journal_file(path)?;
            Ok(false)
        }
        Err(source) => Err(JournalError::FileSync {
            path: path.into(),
            source,
        }),
    }
}

pub(in crate::journal) fn prepare_journal_auxiliary_files(
    path: &Path,
) -> Result<bool, JournalError> {
    let mut wal_name = path.as_os_str().to_os_string();
    wal_name.push("-wal");
    let auxiliary = [path.with_extension("db-log"), wal_name.into()];
    let mut created = false;
    for auxiliary_path in &auxiliary {
        created |= prepare_journal_file(auxiliary_path)?;
    }
    Ok(created)
}

#[cfg(unix)]
pub(in crate::journal) fn prepare_journal_file_platform(
    path: &Path,
) -> std::io::Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
        .open(path)
}

#[cfg(not(unix))]
pub(in crate::journal) fn prepare_journal_file_platform(
    path: &Path,
) -> std::io::Result<std::fs::File> {
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
}

#[cfg(unix)]
pub(in crate::journal) fn validate_journal_parent(path: &Path) -> Result<(), JournalError> {
    use std::os::unix::fs::MetadataExt;
    let metadata = path
        .symlink_metadata()
        .map_err(|source| JournalError::FileSync {
            path: path.into(),
            source,
        })?;
    if !metadata.is_dir()
        || metadata.file_type().is_symlink()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o022 != 0
    {
        return Err(JournalError::InvalidPath(format!(
            "parent {} must be a real directory owned by broker uid with no group/other write bits",
            path.display()
        )));
    }
    Ok(())
}

#[cfg(not(unix))]
pub(in crate::journal) fn validate_journal_parent(path: &Path) -> Result<(), JournalError> {
    let metadata = path
        .symlink_metadata()
        .map_err(|source| JournalError::FileSync {
            path: path.into(),
            source,
        })?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(JournalError::InvalidPath(format!(
            "parent {} is not a real directory",
            path.display()
        )));
    }
    Ok(())
}

#[cfg(unix)]
pub(in crate::journal) fn validate_journal_file(path: &Path) -> Result<(), JournalError> {
    use std::os::unix::fs::MetadataExt;
    let metadata = path
        .symlink_metadata()
        .map_err(|source| JournalError::FileSync {
            path: path.into(),
            source,
        })?;
    if !metadata.is_file()
        || metadata.file_type().is_symlink()
        || metadata.nlink() != 1
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o777 != 0o600
    {
        return Err(JournalError::InvalidPath(format!(
            "{} must be a regular, non-hardlinked, broker-owned 0600 file",
            path.display()
        )));
    }
    Ok(())
}

#[cfg(not(unix))]
pub(in crate::journal) fn validate_journal_file(path: &Path) -> Result<(), JournalError> {
    let metadata = path
        .symlink_metadata()
        .map_err(|source| JournalError::FileSync {
            path: path.into(),
            source,
        })?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(JournalError::InvalidPath(format!(
            "{} is not a regular journal file",
            path.display()
        )));
    }
    Ok(())
}

#[cfg(unix)]
pub(in crate::journal) fn sync_parent(path: &Path) -> Result<(), JournalError> {
    let parent = path.parent().expect("validated parent");
    std::fs::File::open(parent)
        .and_then(|file| file.sync_all())
        .map_err(|source| JournalError::FileSync {
            path: parent.into(),
            source,
        })
}

#[cfg(not(unix))]
pub(in crate::journal) fn sync_parent(_path: &Path) -> Result<(), JournalError> {
    Ok(())
}

#[cfg(unix)]
pub(in crate::journal) fn sync_file(path: &Path) -> Result<(), JournalError> {
    std::fs::OpenOptions::new()
        .read(true)
        .open(path)
        .and_then(|file| file.sync_all())
        .map_err(|source| JournalError::FileSync {
            path: path.into(),
            source,
        })
}

#[cfg(not(unix))]
pub(in crate::journal) fn sync_file(_path: &Path) -> Result<(), JournalError> {
    // Turso's FULL synchronous checkpoint issues FlushFileBuffers on Windows.
    Ok(())
}
