use super::*;

/// Durably publishes immutable bytes without ever replacing an existing path.
///
/// The temporary inode is fully written and synced first. A same-filesystem
/// hard link then creates the destination atomically and fails if another
/// writer already published that name. Removing the temporary link and syncing
/// the parent leaves one durable destination link to the already-synced inode.
pub(crate) fn write_bytes_atomic_new(path: &Path, bytes: &[u8], label: &str) -> CliResult {
    let parent = path
        .parent()
        .ok_or_else(|| CliError::io(format!("{label} path {} has no parent", path.display())))?;
    fs::create_dir_all(parent).map_err(|error| {
        CliError::io(format!(
            "create {label} parent directory {} failed: {error}",
            parent.display()
        ))
    })?;
    let lock_path = write_lock_path(path)?;
    let _write_guard = DurableWriteLockGuard::acquire(&lock_path, label)?;
    match fs::symlink_metadata(path) {
        Ok(_) => {
            return Err(CliError::usage(format!(
                "{label} destination {} already exists; refusing to replace immutable evidence",
                path.display()
            )));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(CliError::io(format!(
                "inspect {label} destination {} failed: {error}",
                path.display()
            )));
        }
    }
    let tmp = temp_path(path)?;
    // Immutable evidence archives must be owner-only (issue #1931).
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    crate::private_fs::harden_open_options(&mut options);
    let mut file = options.open(&tmp).map_err(|error| {
        CliError::io(format!(
            "create-new temporary {label} {} failed: {error}",
            tmp.display()
        ))
    })?;
    crate::private_fs::enforce_private_file_mode(&file, &tmp)?;
    let publish = (|| {
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
        fs::hard_link(&tmp, path).map_err(|error| {
            if error.kind() == std::io::ErrorKind::AlreadyExists {
                CliError::usage(format!(
                    "{label} destination {} was published concurrently; refusing to replace it",
                    path.display()
                ))
            } else {
                CliError::io(format!(
                    "publish immutable {label} {} -> {} failed: {error}",
                    tmp.display(),
                    path.display()
                ))
            }
        })?;
        fs::remove_file(&tmp).map_err(|error| {
            CliError::io(format!(
                "remove temporary link for {label} {} failed after publish: {error}",
                tmp.display()
            ))
        })?;
        sync_parent_dir(parent, label)
    })();
    match publish {
        Ok(()) => Ok(()),
        Err(error) => cleanup_staging_after_error(&tmp, label, error),
    }
}
