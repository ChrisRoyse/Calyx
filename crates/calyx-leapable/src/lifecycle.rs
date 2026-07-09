//! Filesystem lifecycle helpers for Leapable vault RPCs.

use std::fs;
use std::path::{Path, PathBuf};

use calyx_aster::security::SharedVaultContext;
use calyx_aster::verify_restore::{verify_restore, verify_restore_with_value_crypto};
use calyx_core::{CalyxError, Result};
use serde_json::{Value, json};

/// Vault lifecycle error for missing vault directories.
pub const CALYX_LEAPABLE_VAULT_NOT_FOUND: &str = "CALYX_LEAPABLE_VAULT_NOT_FOUND";
/// Vault lifecycle error for target collisions.
pub const CALYX_LEAPABLE_VAULT_ALREADY_EXISTS: &str = "CALYX_LEAPABLE_VAULT_ALREADY_EXISTS";
/// Vault lifecycle error for deleting/restoring over open handles.
pub const CALYX_LEAPABLE_VAULT_OPEN: &str = "CALYX_LEAPABLE_VAULT_OPEN";
/// Vault lifecycle error for unsupported filesystem entries.
pub const CALYX_LEAPABLE_UNSUPPORTED_FILE_TYPE: &str = "CALYX_LEAPABLE_UNSUPPORTED_FILE_TYPE";
const CALYX_LEAPABLE_COPY_FAILED: &str = "CALYX_LEAPABLE_COPY_FAILED";
const CALYX_LEAPABLE_VERIFY_SERIALIZE: &str = "CALYX_LEAPABLE_VERIFY_SERIALIZE";

/// Copy readback summary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CopyStats {
    pub dirs: u64,
    pub files: u64,
    pub bytes: u64,
}

impl CopyStats {
    fn empty() -> Self {
        Self {
            dirs: 0,
            files: 0,
            bytes: 0,
        }
    }

    pub fn to_value(self) -> Value {
        json!({
            "dirs": self.dirs,
            "files": self.files,
            "bytes": self.bytes
        })
    }
}

/// Copies a vault directory to a new target, installing atomically when possible.
pub fn copy_dir_new(source: &Path, target: &Path) -> Result<CopyStats> {
    if !source.is_dir() {
        return Err(lifecycle_error(
            CALYX_LEAPABLE_VAULT_NOT_FOUND,
            format!("source vault dir {} does not exist", source.display()),
            "create or restore the source vault before copying it",
        ));
    }
    if target.exists() {
        return Err(lifecycle_error(
            CALYX_LEAPABLE_VAULT_ALREADY_EXISTS,
            format!("target dir {} already exists", target.display()),
            "choose an unused vault_ref or snapshot_ref",
        ));
    }
    let parent = target.parent().ok_or_else(|| {
        lifecycle_error(
            CALYX_LEAPABLE_COPY_FAILED,
            "copy target has no parent",
            "choose a target under the configured data directory",
        )
    })?;
    fs::create_dir_all(parent).map_err(|error| {
        lifecycle_error(
            CALYX_LEAPABLE_COPY_FAILED,
            format!("create copy parent {}: {error}", parent.display()),
            "free disk space and retry the snapshot/restore operation",
        )
    })?;
    let tmp = temp_sibling(target);
    if tmp.exists() {
        return Err(lifecycle_error(
            CALYX_LEAPABLE_VAULT_ALREADY_EXISTS,
            format!("temporary copy target {} already exists", tmp.display()),
            "remove the stale temporary directory after inspecting it, then retry",
        ));
    }
    let stats = copy_dir_recursive(source, &tmp)?;
    fs::rename(&tmp, target).map_err(|error| {
        let _ = fs::remove_dir_all(&tmp);
        lifecycle_error(
            CALYX_LEAPABLE_COPY_FAILED,
            format!(
                "install copied dir {} -> {}: {error}",
                tmp.display(),
                target.display()
            ),
            "inspect the temporary copy and retry on a writable filesystem",
        )
    })?;
    Ok(stats)
}

/// Removes one already-resolved vault directory.
pub fn remove_dir(path: &Path) -> Result<()> {
    if !path.is_dir() {
        return Err(lifecycle_error(
            CALYX_LEAPABLE_VAULT_NOT_FOUND,
            format!("vault dir {} does not exist", path.display()),
            "list vaults and choose an existing vault_ref",
        ));
    }
    fs::remove_dir_all(path).map_err(|error| {
        lifecycle_error(
            CALYX_LEAPABLE_COPY_FAILED,
            format!("remove vault dir {}: {error}", path.display()),
            "close all readers and retry deletion",
        )
    })
}

/// Runs Aster restore verification and appends pass/fail reasons to the JSON report.
pub fn verify_restore_value(path: &Path) -> Result<Value> {
    let report = verify_restore(path)?;
    restore_report_value(report)
}

/// Runs encrypted Aster restore verification and appends pass/fail reasons.
pub fn verify_restore_value_with_crypto(
    path: &Path,
    context: &SharedVaultContext,
) -> Result<Value> {
    let report = verify_restore_with_value_crypto(path, context)?;
    restore_report_value(report)
}

fn restore_report_value(report: calyx_aster::verify_restore::VerifyRestoreReport) -> Result<Value> {
    let success = report.success();
    let failure_reasons = report.failure_reasons();
    let mut value = serde_json::to_value(&report).map_err(|error| {
        lifecycle_error(
            CALYX_LEAPABLE_VERIFY_SERIALIZE,
            format!("serialize verify_restore report: {error}"),
            "report this bug; verify_restore reports must be serializable",
        )
    })?;
    let object = value.as_object_mut().ok_or_else(|| {
        lifecycle_error(
            CALYX_LEAPABLE_VERIFY_SERIALIZE,
            "verify_restore report did not serialize to an object",
            "report this bug; verify_restore reports must be objects",
        )
    })?;
    object.insert("success".to_string(), Value::Bool(success));
    object.insert("failure_reasons".to_string(), json!(failure_reasons));
    Ok(value)
}

fn copy_dir_recursive(source: &Path, target: &Path) -> Result<CopyStats> {
    fs::create_dir(target).map_err(|error| {
        lifecycle_error(
            CALYX_LEAPABLE_COPY_FAILED,
            format!("create copy target {}: {error}", target.display()),
            "free disk space and retry the copy",
        )
    })?;
    let mut stats = CopyStats::empty();
    stats.dirs += 1;
    for entry in fs::read_dir(source).map_err(|error| {
        lifecycle_error(
            CALYX_LEAPABLE_COPY_FAILED,
            format!("read source dir {}: {error}", source.display()),
            "verify source vault bytes and retry the copy",
        )
    })? {
        let entry = entry.map_err(|error| {
            lifecycle_error(
                CALYX_LEAPABLE_COPY_FAILED,
                format!("read source dir entry: {error}"),
                "verify source vault bytes and retry the copy",
            )
        })?;
        let file_type = entry.file_type().map_err(|error| {
            lifecycle_error(
                CALYX_LEAPABLE_COPY_FAILED,
                format!("stat source path {}: {error}", entry.path().display()),
                "verify source vault bytes and retry the copy",
            )
        })?;
        let child_target = target.join(entry.file_name());
        if file_type.is_dir() {
            let child = copy_dir_recursive(&entry.path(), &child_target)?;
            stats.dirs += child.dirs;
            stats.files += child.files;
            stats.bytes += child.bytes;
        } else if file_type.is_file() {
            let bytes = fs::copy(entry.path(), &child_target).map_err(|error| {
                lifecycle_error(
                    CALYX_LEAPABLE_COPY_FAILED,
                    format!("copy file to {}: {error}", child_target.display()),
                    "free disk space and retry the copy",
                )
            })?;
            stats.files += 1;
            stats.bytes += bytes;
        } else {
            return Err(lifecycle_error(
                CALYX_LEAPABLE_UNSUPPORTED_FILE_TYPE,
                format!(
                    "source path {} is not a regular file or directory",
                    entry.path().display()
                ),
                "remove unsupported filesystem entries from the vault before copying",
            ));
        }
    }
    Ok(stats)
}

fn temp_sibling(target: &Path) -> PathBuf {
    let name = target
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("vault-copy");
    target.with_file_name(format!(".{name}.{}.tmp", std::process::id()))
}

pub fn lifecycle_error(
    code: &'static str,
    message: impl Into<String>,
    remediation: &'static str,
) -> CalyxError {
    CalyxError {
        code,
        message: message.into(),
        remediation,
    }
}
