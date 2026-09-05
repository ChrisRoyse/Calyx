//! Engine configuration.

use std::collections::HashSet;
use std::env;
use std::fs;
use std::path::Path;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::admission::AdmissionParams;
use crate::domain::{self, Domain};
use crate::policy::LocalOnlyPolicy;
use crate::{PolyError, Result};

pub const POLY_CONFIG_LOADED: &str = "CALYX_POLY_CONFIG_LOADED";
const CONFIG_ENV_PREFIX: &str = "POLY_CONFIG_";

/// Top-level configuration for the local-only Polymarket forecast engine.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolyConfig {
    /// Calyx vault root (`$CALYX_HOME`).
    pub calyx_home: String,
    /// The launch domain (data-density-first — crypto).
    pub launch_domain: Domain,
    /// Domains to operate, in build-priority order.
    pub domains: Vec<Domain>,
    /// The panel version to ingest under.
    pub panel_version: u32,
    /// Per-vault content-addressing salt (keeps ids distinct across vaults).
    pub vault_salt: String,
    /// Snapshot poll cadence (seconds) for hot markets.
    pub snapshot_cadence_secs: u64,
    /// Forecast admission/scoring parameters.
    pub admission: AdmissionParams,
    /// Runtime policy that forbids trading and allows only local forecast work.
    pub local_only: LocalOnlyPolicy,
}

impl Default for PolyConfig {
    fn default() -> Self {
        Self {
            calyx_home: ".calyx".to_string(),
            launch_domain: domain::primary_domain(),
            domains: domain::build_order(),
            panel_version: 1,
            vault_salt: "calyx-poly-v1".to_string(),
            snapshot_cadence_secs: 60,
            admission: AdmissionParams::default(),
            local_only: LocalOnlyPolicy::default(),
        }
    }
}

impl PolyConfig {
    /// Loads configuration from a JSON string.
    pub fn from_json(s: &str) -> Result<Self> {
        let config: Self = serde_json::from_str(s).map_err(|err| {
            PolyError::config(
                "POLY_CONFIG_JSON_PARSE",
                format!("parse config JSON: {err}"),
            )
        })?;
        config.validate()?;
        Ok(config)
    }

    /// Serializes configuration to pretty JSON.
    pub fn to_json(&self) -> Result<String> {
        self.validate()?;
        serde_json::to_string_pretty(self).map_err(|err| {
            PolyError::config(
                "POLY_CONFIG_JSON_ENCODE",
                format!("encode config JSON: {err}"),
            )
        })
    }

    /// Loads and validates a complete TOML config string.
    pub fn from_toml_str(s: &str) -> Result<Self> {
        let config: Self = toml::from_str(s).map_err(|err| {
            PolyError::config(
                "POLY_CONFIG_TOML_PARSE",
                format!("parse config TOML: {err}"),
            )
        })?;
        config.validate()?;
        Ok(config)
    }

    /// Loads and validates a complete TOML config file.
    pub fn from_toml_file(path: &Path) -> Result<Self> {
        let contents = fs::read_to_string(path).map_err(|err| {
            PolyError::config(
                "POLY_CONFIG_TOML_READ",
                format!("read config TOML {}: {err}", path.display()),
            )
        })?;
        Self::from_toml_str(&contents)
    }

    /// Loads a TOML config file, applies real process env overrides, then validates it.
    pub fn from_toml_file_with_env(path: &Path) -> Result<Self> {
        Self::from_toml_file_with_env_vars(path, env::vars())
    }

    /// Loads a TOML config file, applies explicit env override pairs, then validates it.
    pub fn from_toml_file_with_env_vars<I, K, V>(path: &Path, vars: I) -> Result<Self>
    where
        I: IntoIterator<Item = (K, V)>,
        K: AsRef<str>,
        V: AsRef<str>,
    {
        let mut config = Self::from_toml_file(path)?;
        config.apply_env_overrides(vars)?;
        config.validate()?;
        Ok(config)
    }

    /// Applies explicit `POLY_CONFIG_*` environment overrides.
    pub fn apply_env_overrides<I, K, V>(&mut self, vars: I) -> Result<()>
    where
        I: IntoIterator<Item = (K, V)>,
        K: AsRef<str>,
        V: AsRef<str>,
    {
        for (key, value) in vars {
            let key = key.as_ref();
            let value = value.as_ref();
            if !key.starts_with(CONFIG_ENV_PREFIX) {
                continue;
            }
            match key {
                "POLY_CONFIG_CALYX_HOME" => self.calyx_home = non_empty_env(key, value)?,
                "POLY_CONFIG_LAUNCH_DOMAIN" => self.launch_domain = parse_domain_env(key, value)?,
                "POLY_CONFIG_DOMAINS" => self.domains = parse_domain_list_env(key, value)?,
                "POLY_CONFIG_PANEL_VERSION" => self.panel_version = parse_env(key, value)?,
                "POLY_CONFIG_VAULT_SALT" => self.vault_salt = non_empty_env(key, value)?,
                "POLY_CONFIG_SNAPSHOT_CADENCE_SECS" => {
                    self.snapshot_cadence_secs = parse_env(key, value)?;
                }
                "POLY_CONFIG_ADMISSION_MIN_P_WIN" => {
                    self.admission.min_p_win = parse_env(key, value)?;
                }
                "POLY_CONFIG_ADMISSION_TARGET_FAR" => {
                    self.admission.target_far = parse_env(key, value)?;
                }
                "POLY_CONFIG_ADMISSION_ALPHA" => self.admission.alpha = parse_env(key, value)?,
                "POLY_CONFIG_ADMISSION_MAX_DAILY_ERROR_SCORE" => {
                    self.admission.max_daily_error_score = parse_env(key, value)?;
                }
                "POLY_CONFIG_ADMISSION_MIN_GROUNDING_ANCHORS" => {
                    self.admission.min_grounding_anchors = parse_env(key, value)?;
                }
                "POLY_CONFIG_ADMISSION_MIN_SOURCE_DERIVED_EVIDENCE" => {
                    self.admission.min_source_derived_evidence = parse_env(key, value)?;
                }
                "POLY_CONFIG_LOCAL_ONLY_ALLOW_FORECAST_AGENTS" => {
                    self.local_only.allow_forecast_agents = parse_env(key, value)?;
                }
                "POLY_CONFIG_LOCAL_ONLY_REQUIRE_INFISICAL_FOR_LLM" => {
                    self.local_only.require_infisical_for_llm = parse_env(key, value)?;
                }
                _ => {
                    return Err(PolyError::config(
                        "POLY_CONFIG_ENV_UNKNOWN",
                        format!("unknown Poly config env override {key}"),
                    ));
                }
            }
        }
        Ok(())
    }

    /// Validates every config field before Poly uses it.
    pub fn validate(&self) -> Result<()> {
        ensure_non_empty(
            "POLY_CONFIG_EMPTY_CALYX_HOME",
            "calyx_home",
            &self.calyx_home,
        )?;
        ensure_non_empty(
            "POLY_CONFIG_EMPTY_VAULT_SALT",
            "vault_salt",
            &self.vault_salt,
        )?;
        ensure(
            self.panel_version > 0,
            "POLY_CONFIG_INVALID_PANEL_VERSION",
            "panel_version must be greater than zero",
        )?;
        ensure(
            self.snapshot_cadence_secs > 0,
            "POLY_CONFIG_INVALID_SNAPSHOT_CADENCE",
            "snapshot_cadence_secs must be greater than zero",
        )?;
        ensure(
            !self.domains.is_empty(),
            "POLY_CONFIG_EMPTY_DOMAINS",
            "domains must contain at least one domain",
        )?;
        ensure(
            self.domains.contains(&self.launch_domain),
            "POLY_CONFIG_LAUNCH_DOMAIN_NOT_ENABLED",
            "launch_domain must be present in domains",
        )?;
        ensure_unique_domains(&self.domains)?;
        validate_admission(&self.admission)?;
        ensure(
            self.local_only.require_infisical_for_llm,
            "POLY_CONFIG_INFISICAL_REQUIRED",
            "local_only.require_infisical_for_llm must remain true",
        )?;
        Ok(())
    }
}

fn validate_admission(params: &AdmissionParams) -> Result<()> {
    ensure_probability("min_p_win", params.min_p_win)?;
    ensure_probability("target_far", params.target_far)?;
    ensure_probability("alpha", params.alpha)?;
    ensure_positive_finite("max_daily_error_score", params.max_daily_error_score)?;
    ensure(
        params.min_grounding_anchors > 0,
        "POLY_CONFIG_INVALID_ADMISSION_PARAM",
        "admission.min_grounding_anchors must be greater than zero",
    )?;
    ensure(
        params.min_source_derived_evidence > 0,
        "POLY_CONFIG_INVALID_ADMISSION_PARAM",
        "admission.min_source_derived_evidence must be greater than zero",
    )
}

fn ensure_unique_domains(domains: &[Domain]) -> Result<()> {
    let mut seen = HashSet::new();
    for domain in domains {
        ensure(
            seen.insert(*domain),
            "POLY_CONFIG_DUPLICATE_DOMAINS",
            format!("duplicate domain {domain:?}"),
        )?;
    }
    Ok(())
}

fn ensure_non_empty(code: &'static str, field: &'static str, value: &str) -> Result<()> {
    ensure(
        !value.trim().is_empty() && !value.contains('\0'),
        code,
        format!("{field} must be non-empty and contain no NUL bytes"),
    )
}

fn ensure_probability(field: &'static str, value: f64) -> Result<()> {
    ensure_range(field, value, 0.0, 1.0)
}

fn ensure_range(field: &'static str, value: f64, min: f64, max: f64) -> Result<()> {
    ensure(
        value.is_finite() && (min..=max).contains(&value),
        "POLY_CONFIG_INVALID_ADMISSION_PARAM",
        format!("admission.{field} must be finite in [{min}, {max}]"),
    )
}

fn ensure_positive_finite(field: &'static str, value: f64) -> Result<()> {
    ensure(
        value.is_finite() && value > 0.0,
        "POLY_CONFIG_INVALID_ADMISSION_PARAM",
        format!("admission.{field} must be finite and greater than zero"),
    )
}

fn ensure(condition: bool, code: &'static str, message: impl Into<String>) -> Result<()> {
    if condition {
        Ok(())
    } else {
        Err(PolyError::config(code, message))
    }
}

fn non_empty_env(name: &str, value: &str) -> Result<String> {
    if value.trim().is_empty() || value.contains('\0') {
        Err(PolyError::config(
            "POLY_CONFIG_ENV_EMPTY",
            format!("{name} must be non-empty and contain no NUL bytes"),
        ))
    } else {
        Ok(value.to_string())
    }
}

fn parse_env<T>(name: &str, value: &str) -> Result<T>
where
    T: FromStr,
    T::Err: std::fmt::Display,
{
    value.parse::<T>().map_err(|err| {
        PolyError::config(
            "POLY_CONFIG_ENV_PARSE",
            format!("parse {name}={value:?}: {err}"),
        )
    })
}

fn parse_domain_list_env(name: &str, value: &str) -> Result<Vec<Domain>> {
    let mut domains = Vec::new();
    for raw in value.split(',') {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Err(PolyError::config(
                "POLY_CONFIG_ENV_PARSE",
                format!("{name} contains an empty domain entry"),
            ));
        }
        domains.push(parse_domain_env(name, trimmed)?);
    }
    Ok(domains)
}

fn parse_domain_env(name: &str, value: &str) -> Result<Domain> {
    let normalized = value.trim().to_ascii_lowercase().replace('-', "_");
    match normalized.as_str() {
        "crypto" => Ok(Domain::Crypto),
        "politics" => Ok(Domain::Politics),
        "sports" => Ok(Domain::Sports),
        "economics" | "macro" => Ok(Domain::Economics),
        "weather" => Ok(Domain::Weather),
        "culture" => Ok(Domain::Culture),
        "geopolitics" => Ok(Domain::Geopolitics),
        "mentions" => Ok(Domain::Mentions),
        "other" => Ok(Domain::Other),
        _ => Err(PolyError::config(
            "POLY_CONFIG_ENV_DOMAIN_INVALID",
            format!("{name} has unsupported domain {value:?}"),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_crypto_launch_and_local_only() {
        let c = PolyConfig::default();
        assert_eq!(c.launch_domain, Domain::Crypto);
        assert_eq!(c.domains[0], Domain::Crypto);
        assert!(c.local_only.allow_forecast_agents);
        assert!(c.local_only.require_infisical_for_llm);
    }

    #[test]
    fn config_json_roundtrips() {
        let c = PolyConfig::default();
        let json = c.to_json().unwrap();
        let back = PolyConfig::from_json(&json).unwrap();
        assert_eq!(back.launch_domain, c.launch_domain);
        assert_eq!(back.admission.min_p_win, c.admission.min_p_win);
        assert_eq!(back.local_only, c.local_only);
    }
}
