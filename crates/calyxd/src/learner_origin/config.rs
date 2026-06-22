use std::path::{Path, PathBuf};

use calyx_core::VaultId;
use serde::Deserialize;

use crate::error::DaemonError;

pub const DEFAULT_ORIGIN_SECRET_ENV: &str = "CALYX_ORIGIN_SHARED_SECRET";
const CALYX_HOME_ENV: &str = "CALYX_HOME";
const DEFAULT_MAX_BODY_BYTES: usize = 256 * 1024;
const MAX_BODY_BYTES_CEILING: usize = 1024 * 1024;

/// Optional `[learner_origin]` block for website Worker-origin writes.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LearnerOriginConfig {
    /// Dedicated learner vault directory. Must not equal the public corpus vault.
    pub vault_path: PathBuf,
    /// Stable learner vault id.
    pub vault_id: VaultId,
    /// Stable salt used for learner-origin content addressing.
    pub vault_salt: String,
    /// Env var containing the shared Worker-origin bearer secret.
    #[serde(default = "default_shared_secret_env")]
    pub shared_secret_env: String,
    /// Maximum accepted JSON body size.
    #[serde(default = "default_max_body_bytes")]
    pub max_body_bytes: usize,
}

impl LearnerOriginConfig {
    pub fn validate(&self, main_raw: &Path, main_resolved: &Path) -> Result<(), DaemonError> {
        if self.vault_salt.trim().is_empty() {
            return Err(DaemonError::config_invalid(
                "learner_origin.vault_salt must not be empty",
            ));
        }
        if self.shared_secret_env.trim().is_empty() {
            return Err(DaemonError::config_invalid(
                "learner_origin.shared_secret_env must name an environment variable",
            ));
        }
        if self.max_body_bytes == 0 || self.max_body_bytes > MAX_BODY_BYTES_CEILING {
            return Err(DaemonError::config_invalid(format!(
                "learner_origin.max_body_bytes {} out of range (must be 1..={MAX_BODY_BYTES_CEILING})",
                self.max_body_bytes
            )));
        }
        let learner_resolved = self.vault_path_resolved();
        if same_path(&self.vault_path, main_raw) || same_path(&learner_resolved, main_resolved) {
            return Err(DaemonError::config_invalid(
                "learner_origin.vault_path must be a dedicated learner vault, not vault_path",
            ));
        }
        Ok(())
    }

    pub fn vault_path_resolved(&self) -> PathBuf {
        resolve_home(&self.vault_path)
    }
}

fn default_shared_secret_env() -> String {
    DEFAULT_ORIGIN_SECRET_ENV.to_string()
}

fn default_max_body_bytes() -> usize {
    DEFAULT_MAX_BODY_BYTES
}

fn resolve_home(path: &Path) -> PathBuf {
    let Some(home) = std::env::var(CALYX_HOME_ENV).ok() else {
        return path.to_path_buf();
    };
    PathBuf::from(
        path.to_string_lossy()
            .replace("${CALYX_HOME}", &home)
            .replace("$CALYX_HOME", &home),
    )
}

fn same_path(left: &Path, right: &Path) -> bool {
    if left == right {
        return true;
    }
    match (std::fs::canonicalize(left), std::fs::canonicalize(right)) {
        (Ok(left), Ok(right)) => left == right,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid() -> LearnerOriginConfig {
        LearnerOriginConfig {
            vault_path: PathBuf::from("/zfs/hot/calyx/learner-origin"),
            vault_id: "01ARZ3NDEKTSV4RRFFQ69G5FAV".parse().unwrap(),
            vault_salt: "learner-origin-salt".to_string(),
            shared_secret_env: DEFAULT_ORIGIN_SECRET_ENV.to_string(),
            max_body_bytes: DEFAULT_MAX_BODY_BYTES,
        }
    }

    #[test]
    fn validates_dedicated_learner_vault() {
        valid()
            .validate(
                Path::new("/zfs/hot/calyx/main"),
                Path::new("/zfs/hot/calyx/main"),
            )
            .expect("dedicated learner vault accepted");
    }

    #[test]
    fn rejects_public_vault_reuse() {
        let mut cfg = valid();
        cfg.vault_path = PathBuf::from("/zfs/hot/calyx/main");
        let error = cfg
            .validate(
                Path::new("/zfs/hot/calyx/main"),
                Path::new("/zfs/hot/calyx/main"),
            )
            .unwrap_err();
        assert_eq!(error.code(), "CALYX_DAEMON_CONFIG_INVALID");
        assert!(error.to_string().contains("dedicated learner vault"));
    }

    #[test]
    fn rejects_empty_secret_env_name() {
        let mut cfg = valid();
        cfg.shared_secret_env.clear();
        assert_eq!(
            cfg.validate(Path::new("/main"), Path::new("/main"))
                .unwrap_err()
                .code(),
            "CALYX_DAEMON_CONFIG_INVALID"
        );
    }
}
