//! Owner-only filesystem contract for authoritative durable vault storage.
//!
//! Every directory and file that makes up a durable vault tree carries
//! security-sensitive derived intelligence, so it must be created owner-only
//! (`0700` directories, `0600` files) regardless of the ambient `umask` of the
//! writing process. Bare `std::fs::create_dir_all` / `File::create` honour the
//! ambient `umask` and never set an explicit mode, which let world-accessible
//! descendants (`0777`/`0666`/`0664`) leak into the tree (issue #1935).
//!
//! This module promotes the hardened technique already proven for retained
//! source bytes (`retained_input::permissions`) into one shared primitive that
//! all vault writers route through: create at an explicit private mode, then
//! read the mode back and fail closed with a structured
//! [`CALYX_VAULT_PERMISSIONS_UNSAFE`] error if the observed mode is not exactly
//! the private mode. File creation is `O_NOFOLLOW` so a swapped-in symlink can
//! never redirect an authoritative write. Non-Unix targets have no owner-only
//! mode contract and fail closed with the same structured error.

use std::fs::{self, File};
use std::io::Write;
use std::path::Path;

use calyx_core::{CalyxError, Result};

/// Structured code raised whenever a vault descendant cannot be proven to be
/// owner-only after a create or migration attempt.
pub const CALYX_VAULT_PERMISSIONS_UNSAFE: &str = "CALYX_VAULT_PERMISSIONS_UNSAFE";

/// Owner-only directory mode (`rwx------`).
pub const PRIVATE_DIRECTORY_MODE: u32 = 0o700;
/// Owner-only file mode (`rw-------`).
pub const PRIVATE_FILE_MODE: u32 = 0o600;

const REMEDIATION: &str = "open the vault write-capable as its owning account so every directory can be hardened to mode 0700 and every file to mode 0600";

fn permission_error(message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: CALYX_VAULT_PERMISSIONS_UNSAFE,
        message: message.into(),
        remediation: REMEDIATION,
    }
}

/// Recursively create `path` and every missing parent as an owner-only
/// (`0700`) directory, then prove the leaf reads back at exactly `0700`.
///
/// Newly created components carry `0700` irrespective of the process `umask`
/// (the explicit `DirBuilder` mode is `0700`, which has no group/other bits for
/// any `umask` to widen). Pre-existing ancestors are left untouched here;
/// hardening a tree written by an older binary is [`harden_tree`]'s job.
pub fn create_private_dir_all(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;

        if let Some(created_leaf) = private_dir_state(path)? {
            if !created_leaf {
                // Directory already present but at the wrong mode: harden it in
                // place and prove the readback rather than trusting the create.
                let dir = open_directory_nofollow(path)?;
                enforce_mode(&dir, path, PRIVATE_DIRECTORY_MODE)?;
                return sync_directory(&dir, path);
            }
            return Ok(());
        }
        let mut builder = fs::DirBuilder::new();
        builder.recursive(true).mode(PRIVATE_DIRECTORY_MODE);
        builder
            .create(path)
            .map_err(|error| permission_error(format!("create {}: {error}", path.display())))?;
        let dir = open_directory_nofollow(path)?;
        enforce_mode(&dir, path, PRIVATE_DIRECTORY_MODE)?;
        crate::fsync::sync_parent(path, "private vault directory")?;
        sync_directory(&dir, path)
    }
    #[cfg(not(unix))]
    {
        Err(unsupported(path))
    }
}

/// Apply owner-only creation flags to a caller-built [`std::fs::OpenOptions`]:
/// create at mode `0600` and refuse to follow a symlinked final component
/// (`O_NOFOLLOW`). A no-op on non-Unix, where the mandatory
/// [`enforce_private_file_mode`] readback fails closed instead.
pub fn harden_open_options(options: &mut std::fs::OpenOptions) {
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

/// Prove an open file handle is exactly owner-only (`0600`), correcting the
/// mode if it drifted and failing closed if the readback disagrees. Call this
/// after opening a freshly created vault file through hardened
/// [`OpenOptions`](std::fs::OpenOptions).
pub fn enforce_private_file_mode(file: &File, path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        enforce_mode(file, path, PRIVATE_FILE_MODE)
    }
    #[cfg(not(unix))]
    {
        let _ = file;
        Err(unsupported(path))
    }
}

/// Open (creating or truncating) an owner-only (`0600`) temp/staging file for a
/// caller that manages its own write + fsync + rename. `O_NOFOLLOW` rejects a
/// swapped-in symlink; a pre-existing regular file is truncated and its mode is
/// enforced back to `0600`. This is the drop-in replacement for the
/// `File::create(tmp)` step of an atomic write.
pub fn open_private_temp_file(path: &Path) -> Result<File> {
    #[cfg(unix)]
    {
        use std::fs::OpenOptions;
        use std::os::unix::fs::OpenOptionsExt;

        let file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .mode(PRIVATE_FILE_MODE)
            .custom_flags(nix::libc::O_CLOEXEC | nix::libc::O_NOFOLLOW)
            .open(path)
            .map_err(|error| {
                permission_error(format!("create private temp {}: {error}", path.display()))
            })?;
        // `.mode()` only applies to a freshly created file; enforce for the
        // truncate-existing case so a stale wrong-mode temp cannot survive.
        enforce_mode(&file, path, PRIVATE_FILE_MODE)?;
        Ok(file)
    }
    #[cfg(not(unix))]
    {
        Err(unsupported(path))
    }
}

/// Atomically publish `bytes` to `path` as an owner-only (`0600`) file: write a
/// private temp sibling, fsync it, rename into place, fsync the parent
/// directory, then read the published file's mode and length back.
pub fn write_private_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| permission_error(format!("{} has no file name", path.display())))?;
    let tmp = path.with_file_name(format!(".{file_name}.{}.tmp", std::process::id()));
    {
        let mut file = open_private_temp_file(&tmp)?;
        file.write_all(bytes)
            .map_err(|error| permission_error(format!("write {}: {error}", tmp.display())))?;
        file.sync_all()
            .map_err(|error| permission_error(format!("fsync {}: {error}", tmp.display())))?;
    }
    fs::rename(&tmp, path).map_err(|error| {
        let _ = fs::remove_file(&tmp);
        permission_error(format!("install {}: {error}", path.display()))
    })?;
    crate::fsync::sync_parent(path, "private vault file")?;
    verify_private_file(path, Some(bytes.len() as u64))
}

/// Read back a published file and prove it is a regular file at exactly `0600`
/// (and, when `expect_len` is supplied, of the expected byte length).
pub fn verify_private_file(path: &Path, expect_len: Option<u64>) -> Result<()> {
    #[cfg(unix)]
    {
        let file = open_file_nofollow(path)?;
        let metadata = file
            .metadata()
            .map_err(|error| permission_error(format!("inspect {}: {error}", path.display())))?;
        if !metadata.is_file() {
            return Err(permission_error(format!(
                "{} is not a regular file",
                path.display()
            )));
        }
        if metadata_mode(&metadata) != PRIVATE_FILE_MODE {
            return Err(permission_error(format!(
                "{} mode readback expected {PRIVATE_FILE_MODE:04o}, observed {:04o}",
                path.display(),
                metadata_mode(&metadata)
            )));
        }
        if let Some(expected) = expect_len
            && metadata.len() != expected
        {
            return Err(permission_error(format!(
                "{} length readback expected {expected}, observed {}",
                path.display(),
                metadata.len()
            )));
        }
        Ok(())
    }
    #[cfg(not(unix))]
    {
        let _ = expect_len;
        Err(unsupported(path))
    }
}

/// Deterministic open-time migration: walk `root` in sorted order and enforce
/// owner-only modes on every descendant directory (`0700`) and file (`0600`),
/// proving each mode by readback. Fails closed on the first descendant that
/// cannot be hardened. Symlinks encountered *inside* the tree are rejected.
///
/// The root itself may legitimately be a symlink: a deployment can hold the
/// vault on one pool and link it into the vault directory. Resolving the root
/// once hardens the link target rather than refusing it. This does not weaken
/// the guard — every descendant is still inspected with `symlink_metadata` and
/// opened `O_NOFOLLOW`, so a swapped-in descendant symlink that would escape
/// the tree is still rejected.
pub fn harden_tree(root: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        if !root.exists() {
            return Ok(());
        }
        let resolved = fs::canonicalize(root).map_err(|error| {
            permission_error(format!("resolve vault root {}: {error}", root.display()))
        })?;
        harden_entry(&resolved)
    }
    #[cfg(not(unix))]
    {
        Err(unsupported(root))
    }
}

#[cfg(unix)]
fn harden_entry(path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| permission_error(format!("inspect {}: {error}", path.display())))?;
    if metadata.file_type().is_symlink() {
        return Err(permission_error(format!(
            "{} is a symlink; refusing to harden a link target",
            path.display()
        )));
    }
    if metadata.is_dir() {
        // Only reopen/chmod/fsync when the mode actually drifted, so opening an
        // already-hardened vault stays a cheap stat+readdir walk.
        if metadata_mode(&metadata) != PRIVATE_DIRECTORY_MODE {
            let dir = open_directory_nofollow(path)?;
            enforce_mode(&dir, path, PRIVATE_DIRECTORY_MODE)?;
            sync_directory(&dir, path)?;
        }
        let mut children = fs::read_dir(path)
            .map_err(|error| permission_error(format!("read dir {}: {error}", path.display())))?
            .map(|entry| {
                entry
                    .map(|entry| entry.path())
                    .map_err(|error| permission_error(format!("read entry: {error}")))
            })
            .collect::<Result<Vec<_>>>()?;
        children.sort();
        for child in children {
            harden_entry(&child)?;
        }
        Ok(())
    } else if metadata.is_file() {
        if metadata_mode(&metadata) != PRIVATE_FILE_MODE {
            let file = open_file_nofollow(path)?;
            enforce_mode(&file, path, PRIVATE_FILE_MODE)?;
            file.sync_all()
                .map_err(|error| permission_error(format!("fsync {}: {error}", path.display())))?;
        }
        Ok(())
    } else {
        Err(permission_error(format!(
            "{} is neither a regular file nor a directory",
            path.display()
        )))
    }
}

// ----- Unix primitives -----------------------------------------------------

#[cfg(unix)]
fn private_dir_state(path: &Path) -> Result<Option<bool>> {
    use std::io::ErrorKind;

    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => Err(
            permission_error(format!("{} is not a real directory", path.display())),
        ),
        Ok(metadata) => Ok(Some(metadata_mode(&metadata) == PRIVATE_DIRECTORY_MODE)),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(permission_error(format!(
            "inspect {}: {error}",
            path.display()
        ))),
    }
}

#[cfg(unix)]
fn open_directory_nofollow(path: &Path) -> Result<File> {
    use std::fs::OpenOptions;
    use std::os::unix::fs::OpenOptionsExt;

    OpenOptions::new()
        .read(true)
        .custom_flags(nix::libc::O_CLOEXEC | nix::libc::O_DIRECTORY | nix::libc::O_NOFOLLOW)
        .open(path)
        .map_err(|error| permission_error(format!("open directory {}: {error}", path.display())))
}

#[cfg(unix)]
fn open_file_nofollow(path: &Path) -> Result<File> {
    use std::fs::OpenOptions;
    use std::os::unix::fs::OpenOptionsExt;

    OpenOptions::new()
        .read(true)
        .custom_flags(nix::libc::O_CLOEXEC | nix::libc::O_NOFOLLOW)
        .open(path)
        .map_err(|error| permission_error(format!("open {}: {error}", path.display())))
}

#[cfg(unix)]
fn sync_directory(dir: &File, path: &Path) -> Result<()> {
    dir.sync_all()
        .map_err(|error| permission_error(format!("sync directory {}: {error}", path.display())))
}

/// Set `path`'s mode to `expected` when it differs, then prove the readback
/// equals `expected` exactly. This is the fail-closed core shared by every
/// create and migration path.
#[cfg(unix)]
pub(crate) fn enforce_mode(file: &File, path: &Path, expected: u32) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let current = mode_of(file, path)?;
    if current != expected {
        file.set_permissions(fs::Permissions::from_mode(expected))
            .map_err(|error| {
                permission_error(format!(
                    "set {} mode {current:04o}->{expected:04o}: {error}",
                    path.display()
                ))
            })?;
    }
    let observed = mode_of(file, path)?;
    if observed != expected {
        return Err(permission_error(format!(
            "{} mode readback expected {expected:04o}, observed {observed:04o}",
            path.display()
        )));
    }
    Ok(())
}

#[cfg(unix)]
fn mode_of(file: &File, path: &Path) -> Result<u32> {
    use std::os::unix::fs::PermissionsExt;

    Ok(file
        .metadata()
        .map_err(|error| permission_error(format!("inspect {}: {error}", path.display())))?
        .permissions()
        .mode()
        & 0o777)
}

#[cfg(unix)]
fn metadata_mode(metadata: &fs::Metadata) -> u32 {
    use std::os::unix::fs::PermissionsExt;

    metadata.permissions().mode() & 0o777
}

#[cfg(not(unix))]
fn unsupported(path: &Path) -> CalyxError {
    permission_error(format!(
        "owner-only vault permissions are unsupported on this platform for {}",
        path.display()
    ))
}

#[cfg(test)]
mod tests;
