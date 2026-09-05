//! Runtime secret contract for Calyx-controlled DeepSeek forecast agents.
//!
//! Secrets are expected to be injected into the process by `infisical run`. This module validates
//! the Poly-specific source metadata and provider config before any caller can build an LLM request.

use std::env;
use std::fmt;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use crate::{PolyError, Result};

pub const POLY_DEEPSEEK_PROJECT_ID: &str = "11b7ea63-6375-43ec-93ed-946505ef683a";
pub const POLY_DEEPSEEK_ENVIRONMENT: &str = "dev";
pub const POLY_DEEPSEEK_SECRET_PATH: &str = "/agents/deepseek";
pub const POLY_DEEPSEEK_API_KEY_NAME: &str = "POLY_DEEPSEEK_API_KEY";
pub const POLY_DEEPSEEK_BASE_URL_NAME: &str = "POLY_DEEPSEEK_BASE_URL";
pub const POLY_DEEPSEEK_MODEL_NAME: &str = "POLY_DEEPSEEK_MODEL";
pub const POLY_DEEPSEEK_BASE_URL: &str = "https://api.deepseek.com";
pub const POLY_DEEPSEEK_MODEL_PRO: &str = "deepseek-v4-pro";
pub const POLY_DEEPSEEK_MODEL_FLASH: &str = "deepseek-v4-flash";

/// Non-secret Infisical coordinates for the Poly DeepSeek provider.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InfisicalDeepSeekSource {
    pub project_id: String,
    pub environment: String,
    pub secret_path: String,
    pub api_key_name: String,
    pub base_url_name: String,
    pub model_name: String,
}

impl Default for InfisicalDeepSeekSource {
    fn default() -> Self {
        Self {
            project_id: POLY_DEEPSEEK_PROJECT_ID.to_string(),
            environment: POLY_DEEPSEEK_ENVIRONMENT.to_string(),
            secret_path: POLY_DEEPSEEK_SECRET_PATH.to_string(),
            api_key_name: POLY_DEEPSEEK_API_KEY_NAME.to_string(),
            base_url_name: POLY_DEEPSEEK_BASE_URL_NAME.to_string(),
            model_name: POLY_DEEPSEEK_MODEL_NAME.to_string(),
        }
    }
}

impl InfisicalDeepSeekSource {
    fn validate(&self) -> Result<()> {
        require_exact(
            "POLY_INFISICAL_PROJECT_MISMATCH",
            "Infisical project ID",
            &self.project_id,
            POLY_DEEPSEEK_PROJECT_ID,
        )?;
        require_exact(
            "POLY_INFISICAL_ENV_MISMATCH",
            "Infisical environment",
            &self.environment,
            POLY_DEEPSEEK_ENVIRONMENT,
        )?;
        require_exact(
            "POLY_INFISICAL_PATH_MISMATCH",
            "Infisical secret path",
            &self.secret_path,
            POLY_DEEPSEEK_SECRET_PATH,
        )?;
        require_exact(
            "POLY_INFISICAL_API_KEY_NAME_MISMATCH",
            "DeepSeek API key secret name",
            &self.api_key_name,
            POLY_DEEPSEEK_API_KEY_NAME,
        )?;
        require_exact(
            "POLY_INFISICAL_BASE_URL_NAME_MISMATCH",
            "DeepSeek base URL secret name",
            &self.base_url_name,
            POLY_DEEPSEEK_BASE_URL_NAME,
        )?;
        require_exact(
            "POLY_INFISICAL_MODEL_NAME_MISMATCH",
            "DeepSeek model secret name",
            &self.model_name,
            POLY_DEEPSEEK_MODEL_NAME,
        )
    }
}

/// Non-secret readback for FSV logs and GitHub issue evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeepSeekSecretMetadata {
    pub project_id: String,
    pub environment: String,
    pub secret_path: String,
    pub api_key_name: String,
    pub key_present: bool,
    pub key_length: usize,
    pub key_has_sk_prefix: bool,
    pub key_sha256_prefix: String,
    pub base_url: String,
    pub model: String,
    pub chat_completions_url: String,
}

/// Validated DeepSeek runtime secrets. Debug intentionally omits the key.
pub struct DeepSeekRuntimeSecrets {
    source: InfisicalDeepSeekSource,
    api_key: Zeroizing<String>,
    base_url: String,
    model: String,
}

impl fmt::Debug for DeepSeekRuntimeSecrets {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DeepSeekRuntimeSecrets")
            .field("source", &self.source)
            .field("api_key", &"<redacted>")
            .field("base_url", &self.base_url)
            .field("model", &self.model)
            .finish()
    }
}

impl DeepSeekRuntimeSecrets {
    /// Load and validate DeepSeek runtime secrets from the current process environment.
    ///
    /// The process must be launched with:
    /// `infisical run --projectId=... --env=dev --path=/agents/deepseek -- <command>`.
    pub fn from_env() -> Result<Self> {
        Self::from_infisical_env(InfisicalDeepSeekSource::default())
    }

    /// Load and validate DeepSeek runtime secrets using explicit non-secret source metadata.
    pub fn from_infisical_env(source: InfisicalDeepSeekSource) -> Result<Self> {
        let api_key = read_required_env(&source.api_key_name)?;
        let base_url = read_required_env(&source.base_url_name)?;
        let model = read_required_env(&source.model_name)?;
        Self::from_values(source, api_key, base_url, model)
    }

    /// Validate explicit values. This is used by FSV edge-case tests and by env loading.
    pub fn from_values(
        source: InfisicalDeepSeekSource,
        api_key: String,
        base_url: String,
        model: String,
    ) -> Result<Self> {
        source.validate()?;
        let api_key = validate_secret_value(&source.api_key_name, api_key)?;
        let base_url = validate_secret_value(&source.base_url_name, base_url)?;
        let model = validate_secret_value(&source.model_name, model)?;

        if !api_key.starts_with("sk-") {
            return Err(PolyError::agent_secret(
                "POLY_DEEPSEEK_SECRET_INVALID_PREFIX",
                format!("{} must start with sk-", source.api_key_name),
            ));
        }
        if base_url != POLY_DEEPSEEK_BASE_URL {
            return Err(PolyError::agent_secret(
                "POLY_DEEPSEEK_BASE_URL_INVALID",
                format!(
                    "{} must equal {}",
                    source.base_url_name, POLY_DEEPSEEK_BASE_URL
                ),
            ));
        }
        if model != POLY_DEEPSEEK_MODEL_PRO && model != POLY_DEEPSEEK_MODEL_FLASH {
            return Err(PolyError::agent_secret(
                "POLY_DEEPSEEK_MODEL_UNSUPPORTED",
                format!(
                    "{} must be {} or {}",
                    source.model_name, POLY_DEEPSEEK_MODEL_PRO, POLY_DEEPSEEK_MODEL_FLASH
                ),
            ));
        }

        Ok(Self {
            source,
            api_key: Zeroizing::new(api_key),
            base_url,
            model,
        })
    }

    /// Build an Authorization header value without leaving the key in ordinary drop memory.
    pub fn bearer_authorization(&self) -> Zeroizing<String> {
        Zeroizing::new(format!("Bearer {}", self.api_key.as_str()))
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    pub fn chat_completions_url(&self) -> String {
        format!("{}/chat/completions", self.base_url)
    }

    /// Non-secret metadata safe for FSV logs and GitHub issues.
    pub fn metadata(&self) -> DeepSeekSecretMetadata {
        DeepSeekSecretMetadata {
            project_id: self.source.project_id.clone(),
            environment: self.source.environment.clone(),
            secret_path: self.source.secret_path.clone(),
            api_key_name: self.source.api_key_name.clone(),
            key_present: true,
            key_length: self.api_key.len(),
            key_has_sk_prefix: self.api_key.starts_with("sk-"),
            key_sha256_prefix: sha256_prefix(self.api_key.as_str(), 12),
            base_url: self.base_url.clone(),
            model: self.model.clone(),
            chat_completions_url: self.chat_completions_url(),
        }
    }
}

fn read_required_env(name: &str) -> Result<String> {
    match env::var(name) {
        Ok(value) => Ok(value),
        Err(env::VarError::NotPresent) => Err(PolyError::agent_secret(
            "POLY_INFISICAL_ENV_MISSING",
            format!("{name} was not injected into the process environment"),
        )),
        Err(env::VarError::NotUnicode(_)) => Err(PolyError::agent_secret(
            "POLY_INFISICAL_ENV_NON_UNICODE",
            format!("{name} was not valid UTF-8"),
        )),
    }
}

fn validate_secret_value(name: &str, value: String) -> Result<String> {
    if value.is_empty() {
        return Err(PolyError::agent_secret(
            "POLY_INFISICAL_SECRET_EMPTY_OR_MISSING",
            format!("{name} was empty"),
        ));
    }
    if value.trim() != value {
        return Err(PolyError::agent_secret(
            "POLY_INFISICAL_SECRET_HAS_WHITESPACE",
            format!("{name} has leading or trailing whitespace"),
        ));
    }
    Ok(value)
}

fn require_exact(code: &str, label: &str, actual: &str, expected: &str) -> Result<()> {
    if actual == expected {
        return Ok(());
    }
    Err(PolyError::agent_secret(
        code,
        format!("{label} must equal {expected}"),
    ))
}

fn sha256_prefix(value: &str, len: usize) -> String {
    let mut hasher = Sha256::new();
    hasher.update(value.as_bytes());
    let digest = hasher.finalize();
    let hex = digest
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    hex[..len].to_string()
}
