//! Owner-only filesystem contract for authoritative retained source bytes.

#[cfg(unix)]
use std::fs::OpenOptions;
use std::fs::{self, File};
use std::io::{ErrorKind, Read};
use std::path::Path;

use calyx_core::{CalyxError, Result};

use super::{
    CALYX_INPUT_BLOB_UNAVAILABLE, CALYX_INPUT_PERMISSIONS_UNSAFE, input_error,
    is_canonical_blob_name,
};

const PRIVATE_DIRECTORY_MODE: u32 = 0o700;
const PRIVATE_FILE_MODE: u32 = 0o600;

pub(super) fn prepare_store_for_write(vault_dir: &Path) -> Result<()> {
    let input_dir = vault_dir.join(super::INPUT_DIR);
    let Some(input_was_private) = private_directory_state(&input_dir)? else {
        return Ok(());
    };
    ensure_private_directory(&input_dir)?;
    let pending_dir = input_dir.join(".pending");
    let pending_was_private = match private_directory_state(&pending_dir)? {
        Some(private) => {
            ensure_private_directory(&pending_dir)?;
            private
        }
        None => true,
    };
    if input_was_private && pending_was_private {
        return Ok(());
    }
    for entry in fs::read_dir(&input_dir).map_err(|error| {
        permission_error(format!(
            "read retained input directory {}: {error}",
            input_dir.display()
        ))
    })? {
        let entry = entry.map_err(|error| {
            permission_error(format!(
                "read retained input entry in {}: {error}",
                input_dir.display()
            ))
        })?;
        if entry.file_name() == ".pending" {
            continue;
        }
        let name = entry.file_name();
        let name = name.to_str().ok_or_else(|| {
            permission_error(format!(
                "retained input entry {} has a non-UTF8 name",
                entry.path().display()
            ))
        })?;
        if !is_canonical_blob_name(name) {
            return Err(permission_error(format!(
                "retained input directory contains unexpected entry {}",
                entry.path().display()
            )));
        }
        harden_private_file(&entry.path())?;
    }
    crate::fsync::sync_dir(&input_dir, "retained input permission migration")
}

pub(super) fn validate_store_for_read(vault_dir: &Path) -> Result<bool> {
    let input_dir = vault_dir.join(super::INPUT_DIR);
    let Some(input_private) = private_directory_state(&input_dir)? else {
        return Ok(false);
    };
    if !input_private {
        return Err(permission_error(format!(
            "retained input directory {} is not mode 0700",
            input_dir.display()
        )));
    }
    let pending_dir = input_dir.join(".pending");
    if let Some(false) = private_directory_state(&pending_dir)? {
        return Err(permission_error(format!(
            "retained input pending directory {} is not mode 0700",
            pending_dir.display()
        )));
    }
    Ok(true)
}

pub(super) fn ensure_private_directory(path: &Path) -> Result<()> {
    #[cfg(unix)]
    let result = {
        use std::os::unix::fs::DirBuilderExt;

        match fs::symlink_metadata(path) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
                return Err(permission_error(format!(
                    "retained input path {} is not a real directory",
                    path.display()
                )));
            }
            Ok(_) => {}
            Err(error) if error.kind() == ErrorKind::NotFound => {
                let mut builder = fs::DirBuilder::new();
                builder.recursive(true).mode(PRIVATE_DIRECTORY_MODE);
                builder.create(path).map_err(|error| {
                    permission_error(format!(
                        "create private directory {}: {error}",
                        path.display()
                    ))
                })?;
                crate::fsync::sync_parent(path, "private retained input directory")?;
            }
            Err(error) => {
                return Err(permission_error(format!(
                    "inspect {}: {error}",
                    path.display()
                )));
            }
        }
        let directory = open_directory_nofollow(path)?;
        enforce_mode(&directory, path, PRIVATE_DIRECTORY_MODE)?;
        directory.sync_all().map_err(|error| {
            permission_error(format!(
                "sync private directory {}: {error}",
                path.display()
            ))
        })?;
        Ok(())
    };
    #[cfg(not(unix))]
    let result = Err(unsupported_permissions(path));
    result
}

pub(super) fn validate_private_directory(path: &Path) -> Result<()> {
    let Some(private) = private_directory_state(path)? else {
        return Err(permission_error(format!(
            "retained input directory {} is absent",
            path.display()
        )));
    };
    if private {
        Ok(())
    } else {
        Err(permission_error(format!(
            "retained input directory {} is not mode 0700",
            path.display()
        )))
    }
}

pub(super) fn create_private_file(path: &Path) -> Result<File> {
    #[cfg(unix)]
    let result = {
        use std::os::unix::fs::OpenOptionsExt;

        let file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(PRIVATE_FILE_MODE)
            .custom_flags(nix::libc::O_CLOEXEC | nix::libc::O_NOFOLLOW)
            .open(path)
            .map_err(|error| {
                permission_error(format!(
                    "create private retained input {}: {error}",
                    path.display()
                ))
            })?;
        enforce_mode(&file, path, PRIVATE_FILE_MODE)?;
        Ok(file)
    };
    #[cfg(not(unix))]
    let result = Err(unsupported_permissions(path));
    result
}

pub(super) fn read_private_file(path: &Path, repair: bool) -> Result<Vec<u8>> {
    let mut file = open_file_nofollow(path).map_err(|error| {
        input_error(
            CALYX_INPUT_BLOB_UNAVAILABLE,
            format!("open retained input {}: {error}", path.display()),
            "restore the named vault input blob from authoritative source bytes",
        )
    })?;
    validate_or_repair_file(&file, path, repair)?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).map_err(|error| {
        input_error(
            CALYX_INPUT_BLOB_UNAVAILABLE,
            format!("read retained input {}: {error}", path.display()),
            "restore the named vault input blob from authoritative source bytes",
        )
    })?;
    Ok(bytes)
}

pub(super) fn read_private_file_if_exists(path: &Path, repair: bool) -> Result<Option<Vec<u8>>> {
    match open_file_nofollow(path) {
        Ok(mut file) => {
            validate_or_repair_file(&file, path, repair)?;
            let mut bytes = Vec::new();
            file.read_to_end(&mut bytes).map_err(|error| {
                permission_error(format!("read retained input {}: {error}", path.display()))
            })?;
            Ok(Some(bytes))
        }
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(permission_error(format!(
            "open retained input {} without following links: {error}",
            path.display()
        ))),
    }
}

pub(super) fn harden_private_file(path: &Path) -> Result<()> {
    let file = open_file_nofollow(path).map_err(|error| {
        permission_error(format!("open retained input {}: {error}", path.display()))
    })?;
    validate_or_repair_file(&file, path, true)
}

fn validate_or_repair_file(file: &File, path: &Path, repair: bool) -> Result<()> {
    let metadata = file.metadata().map_err(|error| {
        permission_error(format!(
            "inspect retained input {}: {error}",
            path.display()
        ))
    })?;
    if !metadata.is_file() {
        return Err(permission_error(format!(
            "retained input {} is not a regular file",
            path.display()
        )));
    }
    if metadata_mode(&metadata)? == PRIVATE_FILE_MODE {
        return Ok(());
    }
    if !repair {
        return Err(permission_error(format!(
            "retained input {} is not mode 0600",
            path.display()
        )));
    }
    enforce_mode(file, path, PRIVATE_FILE_MODE)?;
    file.sync_all().map_err(|error| {
        permission_error(format!(
            "sync retained input permissions {}: {error}",
            path.display()
        ))
    })
}

fn private_directory_state(path: &Path) -> Result<Option<bool>> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            Err(permission_error(format!(
                "retained input path {} is not a real directory",
                path.display()
            )))
        }
        Ok(metadata) => Ok(Some(metadata_mode(&metadata)? == PRIVATE_DIRECTORY_MODE)),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(permission_error(format!(
            "inspect {}: {error}",
            path.display()
        ))),
    }
}

#[cfg(unix)]
fn open_file_nofollow(path: &Path) -> std::io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt;

    OpenOptions::new()
        .read(true)
        .custom_flags(nix::libc::O_CLOEXEC | nix::libc::O_NOFOLLOW)
        .open(path)
}

#[cfg(not(unix))]
fn open_file_nofollow(path: &Path) -> std::io::Result<File> {
    Err(std::io::Error::new(
        ErrorKind::Unsupported,
        unsupported_permissions(path).to_string(),
    ))
}

#[cfg(unix)]
fn open_directory_nofollow(path: &Path) -> Result<File> {
    use std::os::unix::fs::OpenOptionsExt;

    OpenOptions::new()
        .read(true)
        .custom_flags(nix::libc::O_CLOEXEC | nix::libc::O_DIRECTORY | nix::libc::O_NOFOLLOW)
        .open(path)
        .map_err(|error| permission_error(format!("open directory {}: {error}", path.display())))
}

#[cfg(unix)]
fn enforce_mode(file: &File, path: &Path, expected: u32) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let current = file
        .metadata()
        .map_err(|error| permission_error(format!("inspect {}: {error}", path.display())))?
        .permissions()
        .mode()
        & 0o777;
    if current != expected {
        file.set_permissions(fs::Permissions::from_mode(expected))
            .map_err(|error| {
                permission_error(format!(
                    "set {} mode {:04o}->{expected:04o}: {error}",
                    path.display(),
                    current
                ))
            })?;
    }
    let observed = file
        .metadata()
        .map_err(|error| permission_error(format!("read back {} mode: {error}", path.display())))?
        .permissions()
        .mode()
        & 0o777;
    if observed != expected {
        return Err(permission_error(format!(
            "permission readback for {} expected {expected:04o}, observed {observed:04o}",
            path.display()
        )));
    }
    Ok(())
}

#[cfg(not(unix))]
fn enforce_mode(_file: &File, path: &Path, _expected: u32) -> Result<()> {
    Err(unsupported_permissions(path))
}

#[cfg(unix)]
fn metadata_mode(metadata: &fs::Metadata) -> Result<u32> {
    use std::os::unix::fs::PermissionsExt;
    Ok(metadata.permissions().mode() & 0o777)
}

#[cfg(not(unix))]
fn metadata_mode(_metadata: &fs::Metadata) -> Result<u32> {
    Err(unsupported_permissions(Path::new("retained-input")))
}

fn permission_error(message: impl Into<String>) -> CalyxError {
    input_error(
        CALYX_INPUT_PERMISSIONS_UNSAFE,
        message,
        "open the vault write-capable as its owning account to harden retained inputs to directory mode 0700 and file mode 0600",
    )
}

#[cfg(not(unix))]
fn unsupported_permissions(path: &Path) -> CalyxError {
    permission_error(format!(
        "owner-only retained input permissions are unsupported on this platform for {}",
        path.display()
    ))
}
