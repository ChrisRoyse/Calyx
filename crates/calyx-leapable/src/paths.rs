//! Vault-ref path validation and root confinement.

use std::fs;
use std::path::{Path, PathBuf};

use calyx_core::{CalyxError, Result};

/// Fail-closed local error for unsafe Leapable vault refs.
pub const CALYX_LEAPABLE_PATH_INVALID: &str = "CALYX_LEAPABLE_PATH_INVALID";

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
}

/// Resolves `vault_ref` under `data_root`, creating its directory and proving
/// the canonical result remains confined.
pub fn resolve_vault_dir(data_root: &Path, vault_ref: &VaultRef) -> Result<PathBuf> {
    let candidate = data_root.join(vault_ref.as_str());
    fs::create_dir_all(&candidate).map_err(|error| {
        path_error(format!("create vault dir {}: {error}", candidate.display()))
    })?;
    let canonical = candidate.canonicalize().map_err(|error| {
        path_error(format!(
            "canonicalize vault dir {}: {error}",
            candidate.display()
        ))
    })?;
    if !canonical.starts_with(data_root) {
        return Err(path_error(format!(
            "vault dir {} escaped data root {}",
            canonical.display(),
            data_root.display()
        )));
    }
    Ok(canonical)
}

fn path_error(message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: CALYX_LEAPABLE_PATH_INVALID,
        message: message.into(),
        remediation: "send a vault_ref containing only ASCII letters, digits, '.', '_', or '-'",
    }
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
}
