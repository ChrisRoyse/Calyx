//! `CalyxConfig` — the single authoritative runtime configuration for `calyxd`
//! (PH65 · T01).
//!
//! Every daemon tunable (bind address, vault path, VRAM budget, log directory,
//! healthcheck output path, TEI endpoints) is declared here with a documented
//! key and populated from a TOML file (`calyx.toml`). Secrets
//! never appear in the config struct or file — they enter via environment
//! variables or an environment-rendered secret file. Validation is fail-closed:
//! a non-loopback bind address, an out-of-range VRAM budget, a missing key, or
//! a TOML syntax error each yields a stable `CALYX_*` error, never a silent
//! default.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use calyx_core::MtlsConfig;
use serde::Deserialize;

use crate::error::DaemonError;
use crate::learner_origin::LearnerOriginConfig;

/// Upper bound on the VRAM the daemon may budget for Forge, in MiB.
///
/// Conservative ceiling for high-memory CUDA devices; leaves headroom for
/// co-resident GPU services and CUDA context overhead.
const VRAM_BUDGET_MIB_CEILING: u32 = 30_000;

/// Environment variable interpolated into `vault_path` for portability.
const VAULT_PATH_HOME_VAR: &str = "CALYX_HOME";

fn default_bind_addr() -> SocketAddr {
    SocketAddr::from(([127, 0, 0, 1], 7700))
}

fn default_health_log_path() -> PathBuf {
    PathBuf::from("/zfs/hot/logs/calyx-health/latest.json")
}

fn default_healthcheck_timeout_secs() -> u32 {
    30
}

/// Authoritative runtime configuration for the Calyx daemon.
///
/// Constructed only via [`CalyxConfig::from_file`] / [`CalyxConfig::from_toml_str`],
/// both of which run [`CalyxConfig::validate`] before returning. An instance
/// therefore always upholds the invariants: loopback bind address and
/// `0 < vram_budget_mib <= VRAM_BUDGET_MIB_CEILING`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CalyxConfig {
    /// Loopback address the daemon listens on. Default `127.0.0.1:7700`.
    #[serde(default = "default_bind_addr")]
    pub bind_addr: SocketAddr,
    /// Aster vault directory. May contain `$CALYX_HOME` — see
    /// [`CalyxConfig::vault_path_resolved`]. Required (no default).
    pub vault_path: PathBuf,
    /// VRAM budget for Forge, in MiB. Required; must be `1..=30000`.
    pub vram_budget_mib: u32,
    /// Directory for daemon logs. Required (no default).
    pub log_dir: PathBuf,
    /// Path the healthcheck JSON is written to.
    /// Default `/zfs/hot/logs/calyx-health/latest.json`.
    #[serde(default = "default_health_log_path")]
    pub health_log_path: PathBuf,
    /// Text-Embeddings-Inference endpoints (documenting `:8088`/`:8089`/`:8090`).
    #[serde(default)]
    pub tei_endpoints: Vec<String>,
    /// Healthcheck timeout in seconds. Default `30`.
    #[serde(default = "default_healthcheck_timeout_secs")]
    pub healthcheck_timeout_secs: u32,
    /// Optional MCP mTLS block. MCP startup requires this; config parsing keeps
    /// it optional so non-MCP daemon tasks can still load minimal config.
    #[serde(default)]
    pub mcp_mtls: Option<MtlsConfig>,
    /// Optional Worker-only learner-origin API backed by a dedicated Aster vault.
    #[serde(default)]
    pub learner_origin: Option<LearnerOriginConfig>,
}

impl CalyxConfig {
    /// Parse and validate a config from a TOML string.
    ///
    /// A syntax error wraps the underlying parse failure (`CALYX_DAEMON_CONFIG_INVALID`);
    /// a missing required key yields a descriptive `CALYX_DAEMON_CONFIG_INVALID`;
    /// a non-loopback `bind_addr` yields `CALYX_DAEMON_BIND_FAILED`; an
    /// out-of-range `vram_budget_mib` yields `CALYX_FORGE_VRAM_BUDGET`.
    pub fn from_toml_str(text: &str) -> Result<Self, DaemonError> {
        let parsed: CalyxConfig = toml::from_str(text)
            .map_err(|error| DaemonError::config_invalid(format!("parse calyx config: {error}")))?;
        parsed.validate()
    }

    /// Read, parse, and validate a config from a TOML file on disk.
    pub fn from_file(path: &Path) -> Result<Self, DaemonError> {
        let bytes = std::fs::read(path).map_err(|error| {
            DaemonError::config_invalid(format!("read {}: {error}", path.display()))
        })?;
        let text = std::str::from_utf8(&bytes).map_err(|error| {
            DaemonError::config_invalid(format!("{} is not UTF-8: {error}", path.display()))
        })?;
        Self::from_toml_str(text)
    }

    /// Enforce the fail-closed invariants. Consumes and returns `self` so the
    /// only way to obtain a `CalyxConfig` is through a validated path.
    fn validate(self) -> Result<Self, DaemonError> {
        if !self.bind_addr.ip().is_loopback() {
            return Err(DaemonError::bind_failed(format!(
                "bind_addr {} is not loopback; calyxd must bind 127.0.0.1 or [::1]",
                self.bind_addr
            )));
        }
        if self.vram_budget_mib == 0 || self.vram_budget_mib > VRAM_BUDGET_MIB_CEILING {
            return Err(DaemonError::vram_budget(format!(
                "vram_budget_mib {} out of range (must be 1..={VRAM_BUDGET_MIB_CEILING}); \
                 leave headroom for co-resident GPU services",
                self.vram_budget_mib
            )));
        }
        if let Some(mtls) = &self.mcp_mtls {
            validate_mcp_mtls(mtls)?;
        }
        if let Some(origin) = &self.learner_origin {
            origin.validate(&self.vault_path, &self.vault_path_resolved())?;
        }
        Ok(self)
    }

    /// `vault_path` with `$CALYX_HOME` / `${CALYX_HOME}` expanded from the
    /// environment. When the variable is unset the raw path is returned
    /// unchanged, so config files stay portable across dev and production.
    pub fn vault_path_resolved(&self) -> PathBuf {
        resolve_home(&self.vault_path, std::env::var(VAULT_PATH_HOME_VAR).ok())
    }
}

fn validate_mcp_mtls(mtls: &MtlsConfig) -> Result<(), DaemonError> {
    if !mtls.require_client_cert {
        return Err(DaemonError::tls_config_invalid(
            "mcp_mtls.require_client_cert must be true; anonymous MCP clients are refused",
        ));
    }
    if mtls.tls.ca_pem_path.is_none() {
        return Err(DaemonError::tls_config_invalid(
            "mcp_mtls.tls.ca_pem_path is required when client certificates are required",
        ));
    }
    mtls.tls.validate().map_err(|error| {
        DaemonError::tls_config_invalid(format!("{}: {}", error.code, error.message))
    })
}

/// Pure interpolation helper: substitute `home` for `$CALYX_HOME`/`${CALYX_HOME}`
/// when `Some`, otherwise return the path unchanged. Separated from
/// [`CalyxConfig::vault_path_resolved`] so it is testable without mutating the
/// process environment (which is `unsafe` under edition 2024 and racy).
fn resolve_home(path: &Path, home: Option<String>) -> PathBuf {
    match home {
        Some(home) => PathBuf::from(
            path.to_string_lossy()
                .replace("${CALYX_HOME}", &home)
                .replace("$CALYX_HOME", &home),
        ),
        None => path.to_path_buf(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A complete, valid config body used as a baseline by several tests.
    const VALID_TOML: &str = "\
bind_addr = \"127.0.0.1:7700\"
vault_path = \"/zfs/hot/calyx/vault\"
vram_budget_mib = 8192
log_dir = \"/zfs/hot/logs/calyx\"
health_log_path = \"/zfs/hot/logs/calyx-health/latest.json\"
tei_endpoints = [\"http://127.0.0.1:8088\", \"http://127.0.0.1:8089\"]
healthcheck_timeout_secs = 30
";

    #[test]
    fn parses_minimal_valid_config_and_round_trips_fields() {
        // Minimal: only required keys; optional keys fall back to documented defaults.
        let toml = "\
vault_path = \"/data/vault\"
vram_budget_mib = 8192
log_dir = \"/data/logs\"
";
        let config = CalyxConfig::from_toml_str(toml).expect("minimal config parses");
        assert_eq!(config.bind_addr, "127.0.0.1:7700".parse().unwrap());
        assert_eq!(config.vram_budget_mib, 8192);
        assert_eq!(config.vault_path, PathBuf::from("/data/vault"));
        assert_eq!(config.log_dir, PathBuf::from("/data/logs"));
        // Defaults applied for omitted optional keys.
        assert_eq!(
            config.health_log_path,
            PathBuf::from("/zfs/hot/logs/calyx-health/latest.json")
        );
        assert!(config.tei_endpoints.is_empty());
        assert_eq!(config.healthcheck_timeout_secs, 30);
    }

    #[test]
    fn parses_full_config_with_every_key() {
        let config = CalyxConfig::from_toml_str(VALID_TOML).expect("full config parses");
        assert_eq!(config.bind_addr, "127.0.0.1:7700".parse().unwrap());
        assert_eq!(config.vram_budget_mib, 8192);
        assert_eq!(config.tei_endpoints.len(), 2);
        assert_eq!(config.tei_endpoints[0], "http://127.0.0.1:8088");
        assert_eq!(config.healthcheck_timeout_secs, 30);
    }

    #[test]
    fn non_loopback_bind_addr_is_bind_failed() {
        let toml = VALID_TOML.replace("127.0.0.1:7700", "0.0.0.0:7700");
        let error = CalyxConfig::from_toml_str(&toml).unwrap_err();
        assert_eq!(error.code(), "CALYX_DAEMON_BIND_FAILED");
        assert!(error.to_string().contains("0.0.0.0:7700"));
    }

    #[test]
    fn ipv6_loopback_accepted_unspecified_rejected() {
        // [::1] is loopback -> accepted.
        let ok = VALID_TOML.replace("127.0.0.1:7700", "[::1]:7700");
        let config = CalyxConfig::from_toml_str(&ok).expect("[::1] is a valid loopback");
        assert_eq!(config.bind_addr, "[::1]:7700".parse().unwrap());
        // [::] is the unspecified address -> rejected.
        let bad = VALID_TOML.replace("127.0.0.1:7700", "[::]:7700");
        let error = CalyxConfig::from_toml_str(&bad).unwrap_err();
        assert_eq!(error.code(), "CALYX_DAEMON_BIND_FAILED");
    }

    #[test]
    fn zero_vram_budget_rejected_at_parse_time() {
        let toml = VALID_TOML.replace("vram_budget_mib = 8192", "vram_budget_mib = 0");
        let error = CalyxConfig::from_toml_str(&toml).unwrap_err();
        assert_eq!(error.code(), "CALYX_FORGE_VRAM_BUDGET");
        assert!(error.to_string().contains("out of range"));
    }

    #[test]
    fn over_ceiling_vram_budget_rejected_at_parse_time() {
        // 31000 > 30000 ceiling.
        let toml = VALID_TOML.replace("vram_budget_mib = 8192", "vram_budget_mib = 31000");
        let error = CalyxConfig::from_toml_str(&toml).unwrap_err();
        assert_eq!(error.code(), "CALYX_FORGE_VRAM_BUDGET");
    }

    #[test]
    fn ceiling_vram_budget_accepted_one_over_rejected() {
        let at = VALID_TOML.replace("vram_budget_mib = 8192", "vram_budget_mib = 30000");
        assert_eq!(
            CalyxConfig::from_toml_str(&at).unwrap().vram_budget_mib,
            30000
        );
        let over = VALID_TOML.replace("vram_budget_mib = 8192", "vram_budget_mib = 30001");
        assert_eq!(
            CalyxConfig::from_toml_str(&over).unwrap_err().code(),
            "CALYX_FORGE_VRAM_BUDGET"
        );
    }

    #[test]
    fn missing_required_vault_path_is_descriptive_not_panic() {
        let toml = "\
vram_budget_mib = 8192
log_dir = \"/data/logs\"
";
        let error = CalyxConfig::from_toml_str(toml).unwrap_err();
        assert_eq!(error.code(), "CALYX_DAEMON_CONFIG_INVALID");
        assert!(
            error.to_string().contains("vault_path"),
            "error should name the missing key: {error}"
        );
    }

    #[test]
    fn toml_syntax_error_is_wrapped_not_silent_default() {
        let error = CalyxConfig::from_toml_str("this is not = = valid toml").unwrap_err();
        assert_eq!(error.code(), "CALYX_DAEMON_CONFIG_INVALID");
        assert!(error.to_string().contains("parse calyx config"));
    }

    #[test]
    fn unknown_key_rejected_fail_closed() {
        // A typo'd key must error, not be silently ignored.
        let toml = format!("{VALID_TOML}bind_adrr = \"127.0.0.1:9999\"\n");
        let error = CalyxConfig::from_toml_str(&toml).unwrap_err();
        assert_eq!(error.code(), "CALYX_DAEMON_CONFIG_INVALID");
    }

    #[test]
    fn vault_path_interpolates_home_when_set() {
        let path = PathBuf::from("$CALYX_HOME/vault");
        let resolved = resolve_home(&path, Some("/zfs/hot/calyx".to_string()));
        assert_eq!(resolved, PathBuf::from("/zfs/hot/calyx/vault"));
    }

    #[test]
    fn vault_path_interpolates_braced_home_when_set() {
        let path = PathBuf::from("${CALYX_HOME}/vault");
        let resolved = resolve_home(&path, Some("/zfs/hot/calyx".to_string()));
        assert_eq!(resolved, PathBuf::from("/zfs/hot/calyx/vault"));
    }

    #[test]
    fn vault_path_returns_raw_when_home_absent() {
        let path = PathBuf::from("$CALYX_HOME/vault");
        let resolved = resolve_home(&path, None);
        // Unchanged literal path — no silent expansion to empty.
        assert_eq!(resolved, PathBuf::from("$CALYX_HOME/vault"));
    }

    #[test]
    fn mcp_mtls_rejects_missing_ca_when_client_cert_required() {
        let toml = format!(
            "{VALID_TOML}\n[mcp_mtls]\nrequire_client_cert = true\n\n[mcp_mtls.tls]\ncert_pem_path = \"C:/tmp/server.pem\"\nkey_pem_path = \"C:/tmp/server.key\"\n"
        );
        let error = CalyxConfig::from_toml_str(&toml).unwrap_err();
        assert_eq!(error.code(), "CALYX_TLS_CONFIG_INVALID");
        assert!(error.to_string().contains("ca_pem_path"));
    }

    #[test]
    fn mcp_mtls_rejects_optional_client_cert_policy() {
        let toml = format!(
            "{VALID_TOML}\n[mcp_mtls]\nrequire_client_cert = false\n\n[mcp_mtls.tls]\ncert_pem_path = \"C:/tmp/server.pem\"\nkey_pem_path = \"C:/tmp/server.key\"\nca_pem_path = \"C:/tmp/ca.pem\"\n"
        );
        let error = CalyxConfig::from_toml_str(&toml).unwrap_err();
        assert_eq!(error.code(), "CALYX_TLS_CONFIG_INVALID");
        assert!(error.to_string().contains("require_client_cert"));
    }

    #[test]
    fn learner_origin_block_parses_with_defaults() {
        let toml = format!(
            "{VALID_TOML}\n[learner_origin]\nvault_path = \"/zfs/hot/calyx/learner-origin\"\nvault_id = \"01ARZ3NDEKTSV4RRFFQ69G5FAV\"\nvault_salt = \"learner-origin-salt\"\n"
        );
        let config = CalyxConfig::from_toml_str(&toml).expect("origin config parses");
        let origin = config.learner_origin.expect("origin block present");
        assert_eq!(origin.shared_secret_env, "CALYX_ORIGIN_SHARED_SECRET");
        assert_eq!(origin.max_body_bytes, 256 * 1024);
    }

    #[test]
    fn learner_origin_rejects_main_vault_reuse() {
        let toml = format!(
            "{VALID_TOML}\n[learner_origin]\nvault_path = \"/zfs/hot/calyx/vault\"\nvault_id = \"01ARZ3NDEKTSV4RRFFQ69G5FAV\"\nvault_salt = \"learner-origin-salt\"\n"
        );
        let error = CalyxConfig::from_toml_str(&toml).unwrap_err();
        assert_eq!(error.code(), "CALYX_DAEMON_CONFIG_INVALID");
        assert!(error.to_string().contains("dedicated learner vault"));
    }
}
