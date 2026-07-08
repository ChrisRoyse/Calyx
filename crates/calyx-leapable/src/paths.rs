//! Vault-ref path validation and root confinement.

use std::fs;
use std::path::{Path, PathBuf};

use calyx_core::{CalyxError, Result};

use crate::lifecycle::{
    CALYX_LEAPABLE_VAULT_ALREADY_EXISTS, CALYX_LEAPABLE_VAULT_NOT_FOUND, lifecycle_error,
};

/// Fail-closed local error for unsafe Leapable vault refs.
pub const CALYX_LEAPABLE_PATH_INVALID: &str = "CALYX_LEAPABLE_PATH_INVALID";
/// On-disk suffix required by the Leapable storage mapping.
pub const VAULT_DIR_SUFFIX: &str = ".calyx";
/// Snapshot root under the sidecar-provided data directory.
pub const SNAPSHOT_ROOT: &str = "_snapshots";

/// A path-safe, sidecar-provided vault reference.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct VaultRef(String);

impl VaultRef {
    /// Parses a vault ref. Refs are names, not paths: no separators, drives,
    /// parent traversal, whitespace, or empty values.
    pub fn parse(value: &str) -> Result<Self> {
        let valid = !value.is_empty()
            && value.len() <= 128
            && value != "."
            && value != ".."
            && value
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
            && !value.contains("..");
        if valid {
            Ok(Self(value.to_string()))
        } else {
            Err(path_error(format!(
                "vault_ref {value:?} is not a path-safe Leapable vault name"
            )))
        }
    }

    /// Stable string form.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Directory name used under the engine data root.
    pub fn storage_dir_name(&self) -> String {
        if self.0.ends_with(VAULT_DIR_SUFFIX) {
            self.0.clone()
        } else {
            format!("{}{VAULT_DIR_SUFFIX}", self.0)
        }
    }

    /// Converts a storage directory name back to a sidecar-facing ref.
    pub fn from_storage_dir_name(name: &str) -> Result<Option<Self>> {
        let Some(stripped) = name.strip_suffix(VAULT_DIR_SUFFIX) else {
            return Ok(None);
        };
        Self::parse(stripped).map(Some)
    }
}

/// Resolves a new `vault_ref` under `data_root`, failing if it already exists.
pub fn resolve_new_vault_dir(data_root: &Path, vault_ref: &VaultRef) -> Result<PathBuf> {
    let candidate = resolve_vault_target_dir(data_root, vault_ref)?;
    fs::create_dir_all(&candidate).map_err(|error| {
        path_error(format!("create vault dir {}: {error}", candidate.display()))
    })?;
    confined_existing(data_root, &candidate)
}

/// Resolves a not-yet-existing vault target under `data_root`.
pub fn resolve_vault_target_dir(data_root: &Path, vault_ref: &VaultRef) -> Result<PathBuf> {
    let candidate = data_root.join(vault_ref.storage_dir_name());
    if candidate.exists() {
        return Err(vault_exists_error(vault_ref, &candidate));
    }
    confined_parent(data_root, &candidate)?;
    Ok(candidate)
}

/// Resolves an existing `vault_ref` under `data_root`.
pub fn resolve_existing_vault_dir(data_root: &Path, vault_ref: &VaultRef) -> Result<PathBuf> {
    let candidate = data_root.join(vault_ref.storage_dir_name());
    if !candidate.is_dir() {
        return Err(vault_not_found_error(vault_ref, &candidate));
    }
    confined_existing(data_root, &candidate)
}

/// Resolves a snapshot target under `_snapshots/<vault>/<snapshot>`.
pub fn resolve_snapshot_dir(
    data_root: &Path,
    vault_ref: &VaultRef,
    snapshot_ref: &VaultRef,
) -> Result<PathBuf> {
    let parent = data_root
        .join(SNAPSHOT_ROOT)
        .join(vault_ref.storage_dir_name());
    fs::create_dir_all(&parent).map_err(|error| {
        path_error(format!(
            "create snapshot parent {}: {error}",
            parent.display()
        ))
    })?;
    let candidate = parent.join(snapshot_ref.as_str());
    if candidate.exists() {
        return Err(snapshot_exists_error(vault_ref, snapshot_ref));
    }
    confined_parent(data_root, &candidate)?;
    Ok(candidate)
}

/// Resolves an existing snapshot under `_snapshots/<vault>/<snapshot>`.
pub fn resolve_existing_snapshot_dir(
    data_root: &Path,
    vault_ref: &VaultRef,
    snapshot_ref: &VaultRef,
) -> Result<PathBuf> {
    let candidate = data_root
        .join(SNAPSHOT_ROOT)
        .join(vault_ref.storage_dir_name())
        .join(snapshot_ref.as_str());
    if !candidate.is_dir() {
        return Err(snapshot_not_found_error(vault_ref, snapshot_ref));
    }
    confined_existing(data_root, &candidate)
}

/// Lists vault directories under `data_root`.
pub fn list_vault_refs(data_root: &Path) -> Result<Vec<(VaultRef, PathBuf)>> {
    let mut refs = Vec::new();
    for entry in fs::read_dir(data_root)
        .map_err(|error| path_error(format!("read data root {}: {error}", data_root.display())))?
    {
        let entry = entry.map_err(|error| path_error(format!("read data root entry: {error}")))?;
        if !entry
            .file_type()
            .map_err(|error| path_error(format!("stat data root entry: {error}")))?
            .is_dir()
        {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        if name == SNAPSHOT_ROOT {
            continue;
        }
        if let Some(vault_ref) = VaultRef::from_storage_dir_name(&name)? {
            refs.push((vault_ref, confined_existing(data_root, &entry.path())?));
        }
    }
    refs.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(refs)
}

fn confined_existing(data_root: &Path, candidate: &Path) -> Result<PathBuf> {
    let canonical = candidate.canonicalize().map_err(|error| {
        path_error(format!(
            "canonicalize path {}: {error}",
            candidate.display()
        ))
    })?;
    if !canonical.starts_with(data_root) {
        return Err(path_error(format!(
            "path {} escaped data root {}",
            canonical.display(),
            data_root.display()
        )));
    }
    Ok(canonical)
}

fn confined_parent(data_root: &Path, candidate: &Path) -> Result<()> {
    let parent = candidate
        .parent()
        .ok_or_else(|| path_error("candidate has no parent"))?;
    let canonical_parent = parent.canonicalize().map_err(|error| {
        path_error(format!("canonicalize parent {}: {error}", parent.display()))
    })?;
    if !canonical_parent.starts_with(data_root) {
        return Err(path_error(format!(
            "parent {} escaped data root {}",
            canonical_parent.display(),
            data_root.display()
        )));
    }
    Ok(())
}

fn path_error(message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: CALYX_LEAPABLE_PATH_INVALID,
        message: message.into(),
        remediation: "send a vault_ref containing only ASCII letters, digits, '.', '_', or '-'",
    }
}

fn vault_exists_error(vault_ref: &VaultRef, candidate: &Path) -> CalyxError {
    lifecycle_error(
        CALYX_LEAPABLE_VAULT_ALREADY_EXISTS,
        format!(
            "vault_ref {:?} already exists at {}",
            vault_ref.as_str(),
            candidate.display()
        ),
        "choose an unused vault_ref or delete the existing closed vault first",
    )
}

fn vault_not_found_error(vault_ref: &VaultRef, candidate: &Path) -> CalyxError {
    lifecycle_error(
        CALYX_LEAPABLE_VAULT_NOT_FOUND,
        format!(
            "vault_ref {:?} does not exist at {}",
            vault_ref.as_str(),
            candidate.display()
        ),
        "create or restore the vault before opening, deleting, cloning, or verifying it",
    )
}

fn snapshot_exists_error(vault_ref: &VaultRef, snapshot_ref: &VaultRef) -> CalyxError {
    lifecycle_error(
        CALYX_LEAPABLE_VAULT_ALREADY_EXISTS,
        format!(
            "snapshot_ref {:?} already exists for vault_ref {:?}",
            snapshot_ref.as_str(),
            vault_ref.as_str()
        ),
        "choose an unused snapshot_ref for this vault_ref",
    )
}

fn snapshot_not_found_error(vault_ref: &VaultRef, snapshot_ref: &VaultRef) -> CalyxError {
    lifecycle_error(
        CALYX_LEAPABLE_VAULT_NOT_FOUND,
        format!(
            "snapshot_ref {:?} for vault_ref {:?} does not exist",
            snapshot_ref.as_str(),
            vault_ref.as_str()
        ),
        "create the snapshot before restoring it",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vault_ref_rejects_paths() {
        for value in [
            "",
            ".",
            "..",
            "../x",
            "x/y",
            "x\\y",
            "C:\\x",
            "a..b",
            "white space",
        ] {
            assert!(VaultRef::parse(value).is_err(), "{value}");
        }
    }

    #[test]
    fn vault_ref_accepts_slug_like_names() {
        assert_eq!(
            VaultRef::parse("case_01.alpha-beta").unwrap().as_str(),
            "case_01.alpha-beta"
        );
    }

    #[test]
    fn vault_storage_dir_appends_calyx_suffix_once() {
        assert_eq!(
            VaultRef::parse("alpha").unwrap().storage_dir_name(),
            "alpha.calyx"
        );
        assert_eq!(
            VaultRef::parse("alpha.calyx").unwrap().storage_dir_name(),
            "alpha.calyx"
        );
    }
}
