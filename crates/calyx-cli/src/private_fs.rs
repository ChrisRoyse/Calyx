//! Owner-only atomic writer for CLI runtime *evidence* files (issue #1931).
//!
//! Resident readiness/stop JSON, ingest `status.json`, and the warm-panel
//! progress file all record security-sensitive runtime state. Written with a
//! bare `create_dir_all` + `fs::write`, they inherit the operator's ambient
//! `umask` and landed group/world-readable (`0664`/`0666`). This module writes
//! them owner-only and proves it: create a private temp sibling (`0600`,
//! `O_NOFOLLOW`), write, `fsync`, atomically `rename` into place, `fsync` the
//! parent directory, then read the published file's mode and length back,
//! failing closed with a structured [`CALYX_EVIDENCE_PERMISSIONS_UNSAFE`]
//! error. Non-Unix targets have no owner-only mode contract and fail closed
//! with the same structured error.

#[cfg(unix)]
use std::fs;
use std::fs::File;
#[cfg(unix)]
use std::io::Write;
use std::path::Path;

use calyx_core::CalyxError;
use serde::Serialize;

use crate::error::{CliError, CliResult};

/// Structured code raised whenever an evidence file cannot be proven
/// owner-only after being written.
pub(crate) const CALYX_EVIDENCE_PERMISSIONS_UNSAFE: &str = "CALYX_EVIDENCE_PERMISSIONS_UNSAFE";

#[cfg(unix)]
const PRIVATE_DIRECTORY_MODE: u32 = 0o700;
#[cfg(unix)]
const PRIVATE_FILE_MODE: u32 = 0o600;

const REMEDIATION: &str = "write CLI evidence files as the owning account so each is created mode 0600 under a mode-0700 parent";

fn evidence_error(message: impl Into<String>) -> CliError {
    CliError::from(CalyxError {
        code: CALYX_EVIDENCE_PERMISSIONS_UNSAFE,
        message: message.into(),
        remediation: REMEDIATION,
    })
}

/// Recursively create `path` and every missing parent as owner-only (`0700`).
pub(crate) fn create_private_dir_all(path: &Path) -> CliResult {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;

        let mut builder = fs::DirBuilder::new();
        builder.recursive(true).mode(PRIVATE_DIRECTORY_MODE);
        builder
            .create(path)
            .map_err(|error| evidence_error(format!("create {}: {error}", path.display())))?;
        // Harden the leaf even if it pre-existed at a wider mode.
        enforce_dir_mode(path)
    }
    #[cfg(not(unix))]
    {
        Err(unsupported(path))
    }
}

/// Ensure an already-created directory is owner-only (`0700`), proving the
/// mode; fails closed on a symlinked path or on non-Unix. Use for a directory
/// created with a bare `create_dir` whose existence semantics must be kept.
pub(crate) fn harden_private_dir(path: &Path) -> CliResult {
    #[cfg(unix)]
    {
        enforce_dir_mode(path)
    }
    #[cfg(not(unix))]
    {
        Err(unsupported(path))
    }
}

/// Apply owner-only creation flags (mode `0600` + `O_NOFOLLOW`) to a
/// caller-built [`OpenOptions`](std::fs::OpenOptions), so a freshly created (or
/// appended) evidence file is owner-only and never follows a swapped-in
/// symlink. Pair with [`enforce_private_file_mode`] after opening. A no-op on
/// non-Unix, where that readback fails closed instead.
pub(crate) fn harden_open_options(options: &mut std::fs::OpenOptions) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options
            .mode(PRIVATE_FILE_MODE)
            .custom_flags(nix::libc::O_CLOEXEC | nix::libc::O_NOFOLLOW);
    }
    #[cfg(not(unix))]
    {
        let _ = options;
    }
}

/// Prove an open handle is exactly owner-only (`0600`), correcting a drifted
/// mode and re-reading it back; fails closed on mismatch or on non-Unix.
pub(crate) fn enforce_private_file_mode(file: &File, path: &Path) -> CliResult {
    #[cfg(unix)]
    {
        enforce_file_mode(file, path)
    }
    #[cfg(not(unix))]
    {
        let _ = file;
        Err(unsupported(path))
    }
}

/// Serialize `value` as pretty JSON and publish it owner-only and atomically.
pub(crate) fn write_private_json(path: &Path, value: &impl Serialize) -> CliResult {
    let bytes = serde_json::to_vec_pretty(value)
        .map_err(|error| CliError::runtime(format!("serialize {}: {error}", path.display())))?;
    write_private_bytes(path, &bytes)
}

/// Publish `bytes` to `path` as an owner-only (`0600`) file under a mode-`0700`
/// parent, atomically, then read the mode and length back.
pub(crate) fn write_private_bytes(path: &Path, bytes: &[u8]) -> CliResult {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        create_private_dir_all(parent)?;
    }
    #[cfg(unix)]
    {
        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| evidence_error(format!("{} has no file name", path.display())))?;
        let tmp = path.with_file_name(format!(".{file_name}.{}.tmp", std::process::id()));
        let result = write_and_publish(&tmp, path, bytes);
        if result.is_err() {
            let _ = fs::remove_file(&tmp);
        }
        result
    }
    #[cfg(not(unix))]
    {
        let _ = bytes;
        Err(unsupported(path))
    }
}

#[cfg(unix)]
fn write_and_publish(tmp: &Path, path: &Path, bytes: &[u8]) -> CliResult {
    use std::os::unix::fs::OpenOptionsExt;

    let mut file = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(PRIVATE_FILE_MODE)
        .custom_flags(nix::libc::O_CLOEXEC | nix::libc::O_NOFOLLOW)
        .open(tmp)
        .map_err(|error| {
            evidence_error(format!("create evidence temp {}: {error}", tmp.display()))
        })?;
    enforce_file_mode(&file, tmp)?;
    file.write_all(bytes)
        .map_err(|error| evidence_error(format!("write {}: {error}", tmp.display())))?;
    file.sync_all()
        .map_err(|error| evidence_error(format!("fsync {}: {error}", tmp.display())))?;
    drop(file);
    fs::rename(tmp, path)
        .map_err(|error| evidence_error(format!("install {}: {error}", path.display())))?;
    sync_parent_dir(path)?;
    verify_published(path, bytes.len() as u64)
}

#[cfg(unix)]
fn verify_published(path: &Path, expect_len: u64) -> CliResult {
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

    let file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(nix::libc::O_CLOEXEC | nix::libc::O_NOFOLLOW)
        .open(path)
        .map_err(|error| evidence_error(format!("reopen {}: {error}", path.display())))?;
    let metadata = file
        .metadata()
        .map_err(|error| evidence_error(format!("inspect {}: {error}", path.display())))?;
    if !metadata.is_file() {
        return Err(evidence_error(format!(
            "{} is not a regular file",
            path.display()
        )));
    }
    let mode = metadata.permissions().mode() & 0o777;
    if mode != PRIVATE_FILE_MODE {
        return Err(evidence_error(format!(
            "{} mode readback expected {PRIVATE_FILE_MODE:04o}, observed {mode:04o}",
            path.display()
        )));
    }
    if metadata.len() != expect_len {
        return Err(evidence_error(format!(
            "{} length readback expected {expect_len}, observed {}",
            path.display(),
            metadata.len()
        )));
    }
    Ok(())
}

#[cfg(unix)]
fn enforce_file_mode(file: &File, path: &Path) -> CliResult {
    use std::os::unix::fs::PermissionsExt;

    let current = file
        .metadata()
        .map_err(|error| evidence_error(format!("inspect {}: {error}", path.display())))?
        .permissions()
        .mode()
        & 0o777;
    if current != PRIVATE_FILE_MODE {
        file.set_permissions(fs::Permissions::from_mode(PRIVATE_FILE_MODE))
            .map_err(|error| {
                evidence_error(format!("set {} mode 0600: {error}", path.display()))
            })?;
    }
    let observed = file
        .metadata()
        .map_err(|error| evidence_error(format!("read back {} mode: {error}", path.display())))?
        .permissions()
        .mode()
        & 0o777;
    if observed != PRIVATE_FILE_MODE {
        return Err(evidence_error(format!(
            "{} mode readback expected {PRIVATE_FILE_MODE:04o}, observed {observed:04o}",
            path.display()
        )));
    }
    Ok(())
}

#[cfg(unix)]
fn enforce_dir_mode(path: &Path) -> CliResult {
    use std::os::unix::fs::PermissionsExt;

    let metadata = fs::symlink_metadata(path)
        .map_err(|error| evidence_error(format!("inspect {}: {error}", path.display())))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(evidence_error(format!(
            "{} is not a real directory",
            path.display()
        )));
    }
    if metadata.permissions().mode() & 0o777 != PRIVATE_DIRECTORY_MODE {
        fs::set_permissions(path, fs::Permissions::from_mode(PRIVATE_DIRECTORY_MODE)).map_err(
            |error| evidence_error(format!("set {} mode 0700: {error}", path.display())),
        )?;
    }
    Ok(())
}

#[cfg(unix)]
fn sync_parent_dir(path: &Path) -> CliResult {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        let dir = File::open(parent).map_err(|error| {
            evidence_error(format!("open parent {}: {error}", parent.display()))
        })?;
        dir.sync_all().map_err(|error| {
            evidence_error(format!("fsync parent {}: {error}", parent.display()))
        })?;
    }
    Ok(())
}

#[cfg(not(unix))]
fn unsupported(path: &Path) -> CliError {
    evidence_error(format!(
        "owner-only evidence permissions are unsupported on this platform for {}",
        path.display()
    ))
}

#[cfg(test)]
mod tests;
