//! Vault data residency — governance by construction (PRD `30 §4`, axiom A33).
//!
//! A vault may pin its storage location to a single dataset root. Once pinned,
//! the pin is immutable and persisted in the vault's config sidecar
//! (`residency.json`), so it survives restart and can be read back as a
//! first-class governance property. Any write or copy whose target lands
//! outside the pinned dataset fails closed with `CALYX_RESIDENCY_VIOLATION`
//! unless the pin explicitly permits off-dataset placement. Enforcement is
//! lexical (deterministic, fail-closed on `..` escapes) and does not depend on
//! the target existing yet.

use std::path::{Component, Path, PathBuf};

use calyx_core::{CalyxError, Result};
use serde::{Deserialize, Serialize};

/// Sidecar file, relative to the vault root, holding the residency pin.
pub const RESIDENCY_SIDECAR: &str = "residency.json";

/// A vault's pinned storage location plus its off-dataset policy.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Residency {
    /// Absolute dataset root the vault's data is pinned to.
    pub dataset_root: PathBuf,
    /// When true, copies/writes outside `dataset_root` are permitted (policy
    /// exception). Defaults to false — fail closed.
    #[serde(default)]
    pub allow_off_dataset: bool,
}

impl Residency {
    /// Pins residency to `dataset_root` with the strict (fail-closed) policy.
    pub fn pin(dataset_root: impl Into<PathBuf>) -> Self {
        Self {
            dataset_root: normalize_lexical(&dataset_root.into()),
            allow_off_dataset: false,
        }
    }

    /// Pins residency, explicitly allowing off-dataset placement (policy
    /// exception per PRD `30 §4`).
    pub fn pin_allowing_off_dataset(dataset_root: impl Into<PathBuf>) -> Self {
        Self {
            dataset_root: normalize_lexical(&dataset_root.into()),
            allow_off_dataset: true,
        }
    }

    /// Authorizes a target path against the pin. `Ok(())` when the target is
    /// within the pinned dataset (or the policy permits off-dataset); otherwise
    /// a fail-closed `CALYX_RESIDENCY_VIOLATION`.
    pub fn authorize(&self, target: &Path) -> Result<()> {
        if self.allow_off_dataset {
            return Ok(());
        }
        let target = normalize_lexical(target);
        if target.starts_with(&self.dataset_root) {
            Ok(())
        } else {
            Err(residency_violation(format!(
                "target {} is outside the pinned residency dataset {}",
                target.display(),
                self.dataset_root.display()
            )))
        }
    }

    /// Redaction-safe blake3 digest (hex) of a path's normalized form. The
    /// Ledger forbids raw paths in payloads (they could be secrets), so audit
    /// entries reference paths by this verifiable digest instead.
    pub fn path_digest(path: &Path) -> String {
        blake3::hash(normalize_lexical(path).to_string_lossy().as_bytes())
            .to_hex()
            .to_string()
    }

    /// Digest of the pinned dataset root (see [`Residency::path_digest`]).
    pub fn dataset_root_digest(&self) -> String {
        Self::path_digest(&self.dataset_root)
    }

    /// A compact, leak-free governance subject label for the Ledger.
    pub fn audit_subject(&self) -> Vec<u8> {
        format!("residency:{}", self.dataset_root_digest()).into_bytes()
    }

    /// Persists the pin to `<root>/residency.json`. Idempotent when the same pin
    /// already exists; a different existing pin fails closed
    /// (`CALYX_RESIDENCY_PIN_CONFLICT`) because residency is immutable.
    pub fn persist(&self, vault_root: &Path) -> Result<()> {
        if let Some(existing) = Self::load(vault_root)? {
            if &existing == self {
                return Ok(());
            }
            return Err(pin_conflict(format!(
                "vault already pinned to {} (allow_off_dataset={}); refusing to re-pin to {} (allow_off_dataset={})",
                existing.dataset_root.display(),
                existing.allow_off_dataset,
                self.dataset_root.display(),
                self.allow_off_dataset
            )));
        }
        let path = vault_root.join(RESIDENCY_SIDECAR);
        let tmp = vault_root.join(format!("{RESIDENCY_SIDECAR}.tmp"));
        let bytes = serde_json::to_vec_pretty(self)
            .map_err(|error| residency_corrupt(format!("encode residency pin: {error}")))?;
        std::fs::write(&tmp, &bytes)
            .map_err(|error| residency_io(format!("write residency sidecar: {error}")))?;
        std::fs::rename(&tmp, &path)
            .map_err(|error| residency_io(format!("commit residency sidecar: {error}")))?;
        Ok(())
    }

    /// Reads the residency pin from `<root>/residency.json`, if present.
    pub fn load(vault_root: &Path) -> Result<Option<Self>> {
        let path = vault_root.join(RESIDENCY_SIDECAR);
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(residency_io(format!("read residency sidecar: {error}")));
            }
        };
        let residency: Self = serde_json::from_slice(&bytes)
            .map_err(|error| residency_corrupt(format!("decode residency pin: {error}")))?;
        Ok(Some(residency))
    }

    /// Fails closed if any tiering tier-root lands outside the pinned dataset
    /// (the real off-dataset copy path that exists today). A permissive pin
    /// (`allow_off_dataset`) skips the check.
    pub fn enforce_tier_roots(&self, tier_roots: &[PathBuf]) -> Result<()> {
        if self.allow_off_dataset {
            return Ok(());
        }
        for root in tier_roots {
            self.authorize(root).map_err(|_| {
                residency_violation(format!(
                    "tiering tier-root {} lands outside the pinned residency dataset {}",
                    normalize_lexical(root).display(),
                    self.dataset_root.display()
                ))
            })?;
        }
        Ok(())
    }
}

/// Lexically normalizes a path to an absolute, `.`/`..`-folded form without
/// touching the filesystem (so not-yet-created targets normalize correctly and
/// `..` escapes are resolved rather than trusted).
fn normalize_lexical(path: &Path) -> PathBuf {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("/"))
            .join(path)
    };
    let mut out: Vec<Component> = Vec::new();
    for component in absolute.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if matches!(out.last(), Some(Component::Normal(_))) {
                    out.pop();
                } else if !matches!(out.last(), Some(Component::RootDir | Component::Prefix(_))) {
                    out.push(component);
                }
            }
            other => out.push(other),
        }
    }
    out.iter().collect()
}

fn residency_violation(message: impl Into<String>) -> CalyxError {
    residency_error("CALYX_RESIDENCY_VIOLATION", message)
}

fn pin_conflict(message: impl Into<String>) -> CalyxError {
    residency_error("CALYX_RESIDENCY_PIN_CONFLICT", message)
}

fn residency_corrupt(message: impl Into<String>) -> CalyxError {
    residency_error("CALYX_RESIDENCY_CORRUPT", message)
}

fn residency_io(message: impl Into<String>) -> CalyxError {
    residency_error("CALYX_RESIDENCY_IO", message)
}

fn residency_error(code: &'static str, message: impl Into<String>) -> CalyxError {
    CalyxError {
        code,
        message: message.into(),
        remediation: "keep vault writes/copies within the pinned residency dataset, or set an explicit off-dataset policy",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn in_dataset_target_is_authorized() {
        let r = Residency::pin("/data/vault");
        assert!(
            r.authorize(Path::new("/data/vault/cf/base/0001.sst"))
                .is_ok()
        );
    }

    #[test]
    fn off_dataset_target_fails_closed() {
        let r = Residency::pin("/data/vault");
        let err = r.authorize(Path::new("/tmp/exfil.sst")).unwrap_err();
        assert_eq!(err.code, "CALYX_RESIDENCY_VIOLATION");
    }

    #[test]
    fn parent_dir_escape_is_resolved_then_rejected() {
        let r = Residency::pin("/data/vault");
        // /data/vault/../other normalizes to /data/other -> outside.
        let err = r
            .authorize(Path::new("/data/vault/../other/file"))
            .unwrap_err();
        assert_eq!(err.code, "CALYX_RESIDENCY_VIOLATION");
    }

    #[test]
    fn sibling_prefix_is_not_treated_as_inside() {
        let r = Residency::pin("/data/vault");
        // /data/vault-evil must not be considered inside /data/vault.
        let err = r.authorize(Path::new("/data/vault-evil/x")).unwrap_err();
        assert_eq!(err.code, "CALYX_RESIDENCY_VIOLATION");
    }

    #[test]
    fn permissive_policy_allows_off_dataset() {
        let r = Residency::pin_allowing_off_dataset("/data/vault");
        assert!(r.authorize(Path::new("/tmp/anywhere")).is_ok());
    }

    #[test]
    fn tier_roots_outside_dataset_fail_closed() {
        let r = Residency::pin("/data/vault");
        let err = r
            .enforce_tier_roots(&[
                PathBuf::from("/data/vault/cold"),
                PathBuf::from("/mnt/other"),
            ])
            .unwrap_err();
        assert_eq!(err.code, "CALYX_RESIDENCY_VIOLATION");
        assert!(
            r.enforce_tier_roots(&[PathBuf::from("/data/vault/cold")])
                .is_ok()
        );
    }
}
