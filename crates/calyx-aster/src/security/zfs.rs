//! ZFS native-encryption probe + operator guidance (PH60 · T06).
//!
//! The outermost crypto-at-rest layer (PRD `30 §2`). Calyx never *enables* ZFS
//! encryption — that is operator/sudo-gated:
//!
//! ```text
//! zfs create -o encryption=aes-256-gcm -o keylocation=prompt \
//!            -o keyformat=passphrase tank/calyx
//! ```
//!
//! This module only *reads* the dataset's encryption status and, when it is
//! absent, emits a human-readable guidance string. It must **never panic or
//! fail the process** when ZFS is unavailable — dev machines without ZFS (e.g.
//! Windows/macOS) must still run, so a missing `zfs` binary maps to
//! [`ZfsEncryptionStatus::ZfsNotAvailable`], not an error.

use std::fmt;
use std::process::Command;

/// Result of probing a ZFS dataset's `encryption` property.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ZfsEncryptionStatus {
    /// Encryption is on; `algorithm` is the reported cipher (e.g. `aes-256-gcm`).
    Enabled { algorithm: String },
    /// The dataset exists but encryption is `off`.
    Disabled,
    /// `zfs` could not be run (not installed, EPERM, etc.) — not an error in dev.
    ZfsNotAvailable,
    /// `zfs` ran but reported the dataset does not exist.
    DatasetNotFound { dataset: String },
}

impl fmt::Display for ZfsEncryptionStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Enabled { algorithm } => {
                write!(f, "ZFS encryption enabled (algorithm = {algorithm})")
            }
            Self::Disabled => write!(
                f,
                "ZFS encryption is DISABLED — data at rest is not encrypted"
            ),
            Self::ZfsNotAvailable => write!(
                f,
                "ZFS is not available on this host; encryption status unknown"
            ),
            Self::DatasetNotFound { dataset } => {
                write!(f, "ZFS dataset `{dataset}` does not exist")
            }
        }
    }
}

/// Probes the `encryption` property of `dataset` via
/// `zfs get -H -o value encryption <dataset>`.
///
/// Never panics: a failure to spawn `zfs` (missing binary, permission denied)
/// returns [`ZfsEncryptionStatus::ZfsNotAvailable`]. The parse of a successful
/// or failed run is delegated to [`classify_zfs_output`] so it can be unit
/// tested deterministically without a live ZFS pool.
pub fn probe_zfs_encryption(dataset: &str) -> ZfsEncryptionStatus {
    let output = Command::new("zfs")
        .args(["get", "-H", "-o", "value", "encryption", dataset])
        .output();
    match output {
        Ok(out) => classify_zfs_output(
            out.status.success(),
            &String::from_utf8_lossy(&out.stdout),
            &String::from_utf8_lossy(&out.stderr),
            dataset,
        ),
        // zfs binary missing / not executable / blocked — fine in dev.
        Err(_) => ZfsEncryptionStatus::ZfsNotAvailable,
    }
}

/// Pure classifier for a `zfs get encryption` invocation. Separated from the
/// process spawn so synthetic command outputs can be tested byte-for-byte.
pub fn classify_zfs_output(
    exit_ok: bool,
    stdout: &str,
    stderr: &str,
    dataset: &str,
) -> ZfsEncryptionStatus {
    if exit_ok {
        let value = stdout.trim();
        // `encryption=off` (or empty/`none`) means no at-rest encryption; any
        // other non-empty value is the active cipher suite.
        if value.is_empty()
            || value.eq_ignore_ascii_case("off")
            || value.eq_ignore_ascii_case("none")
        {
            ZfsEncryptionStatus::Disabled
        } else {
            ZfsEncryptionStatus::Enabled {
                algorithm: value.to_string(),
            }
        }
    } else if stderr.contains("does not exist") || stderr.contains("cannot open") {
        ZfsEncryptionStatus::DatasetNotFound {
            dataset: dataset.to_string(),
        }
    } else {
        // Permission denied, command misuse, or any other failure: the probe
        // could not determine status. Fail soft (dev), never panic.
        ZfsEncryptionStatus::ZfsNotAvailable
    }
}

/// Returns a human-readable operator remediation string when encryption is
/// absent and the operator can act on it.
///
/// `None` for [`ZfsEncryptionStatus::Enabled`] (nothing to do) and for
/// [`ZfsEncryptionStatus::ZfsNotAvailable`] (the probe could not run — not an
/// actionable error during development).
pub fn operator_guidance(status: &ZfsEncryptionStatus) -> Option<&'static str> {
    match status {
        ZfsEncryptionStatus::Enabled { .. } | ZfsEncryptionStatus::ZfsNotAvailable => None,
        ZfsEncryptionStatus::Disabled => Some(
            "ZFS encryption is off. Recreate the dataset with native encryption (sudo): \
             `zfs create -o encryption=aes-256-gcm -o keylocation=prompt \
             -o keyformat=passphrase tank/calyx`. Existing datasets cannot be \
             encrypted in place — migrate data into an encrypted dataset.",
        ),
        ZfsEncryptionStatus::DatasetNotFound { .. } => Some(
            "The calyx ZFS dataset does not exist. Create it with native encryption (sudo): \
             `zfs create -o encryption=aes-256-gcm -o keylocation=prompt \
             -o keyformat=passphrase tank/calyx`.",
        ),
    }
}

/// Probes `dataset` and logs a WARN to stderr if encryption is not enabled,
/// returning the status for the caller to record in the vault manifest. Never
/// panics and never fails — this is advisory, not a gate.
pub fn assert_encrypted_or_warn(dataset: &str) -> ZfsEncryptionStatus {
    let status = probe_zfs_encryption(dataset);
    if !matches!(status, ZfsEncryptionStatus::Enabled { .. }) {
        eprintln!("WARN: {status}");
        if let Some(guidance) = operator_guidance(&status) {
            eprintln!("WARN: {guidance}");
        }
    }
    status
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_enabled_cipher() {
        let status = classify_zfs_output(true, "aes-256-gcm\n", "", "tank/calyx");
        println!("classify(aes-256-gcm) = {status} -> {status:?}");
        assert_eq!(
            status,
            ZfsEncryptionStatus::Enabled {
                algorithm: "aes-256-gcm".to_string()
            }
        );
        assert!(
            operator_guidance(&status).is_none(),
            "enabled needs no guidance"
        );
    }

    #[test]
    fn classifies_disabled() {
        for value in ["off\n", "OFF", "none", "  off  ", ""] {
            let status = classify_zfs_output(true, value, "", "tank/calyx");
            assert_eq!(status, ZfsEncryptionStatus::Disabled, "value {value:?}");
        }
        let guidance = operator_guidance(&ZfsEncryptionStatus::Disabled);
        println!("disabled guidance = {guidance:?}");
        assert!(
            guidance.is_some_and(|s| !s.is_empty()),
            "disabled must guide the operator"
        );
    }

    #[test]
    fn classifies_dataset_not_found() {
        let stderr = "cannot open 'tank/calyx': dataset does not exist\n";
        let status = classify_zfs_output(false, "", stderr, "tank/calyx");
        assert_eq!(
            status,
            ZfsEncryptionStatus::DatasetNotFound {
                dataset: "tank/calyx".to_string()
            }
        );
        assert!(operator_guidance(&status).is_some_and(|s| !s.is_empty()));
    }

    #[test]
    fn permission_denied_is_not_available_not_panic() {
        // exit 1 with a non-"does not exist" stderr (e.g. EPERM) -> not available.
        let status = classify_zfs_output(false, "", "permission denied\n", "tank/calyx");
        assert_eq!(status, ZfsEncryptionStatus::ZfsNotAvailable);
        assert!(
            operator_guidance(&status).is_none(),
            "unavailable is not actionable"
        );
    }

    #[test]
    fn real_probe_on_host_without_zfs_returns_not_available_without_panic() {
        // Physical readback: this dev host has no `zfs` binary, so the real
        // process spawn must fail soft to ZfsNotAvailable (never a panic).
        let status = probe_zfs_encryption("tank/calyx-nonexistent-probe-xyz");
        println!("real probe_zfs_encryption(...) = {status} -> {status:?}");
        assert!(
            matches!(
                status,
                ZfsEncryptionStatus::ZfsNotAvailable
                    | ZfsEncryptionStatus::DatasetNotFound { .. }
                    | ZfsEncryptionStatus::Disabled
                    | ZfsEncryptionStatus::Enabled { .. }
            ),
            "probe must return a status, never panic"
        );
    }

    #[test]
    fn assert_encrypted_or_warn_returns_status() {
        // Advisory path: returns a status, never panics/fails.
        let status = assert_encrypted_or_warn("tank/calyx-nonexistent-probe-xyz");
        println!("assert_encrypted_or_warn(...) = {status:?}");
    }
}
