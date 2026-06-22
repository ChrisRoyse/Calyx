use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{SlotId, SystemClock, VaultId};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::config::LearnerOriginConfig;
use super::metrics::OriginMetrics;
use super::model::{
    ENDPOINT_DECIDE, ENDPOINT_OUTCOMES, ENDPOINT_SIGNALS, KIND_DECISION, KIND_OUTCOME,
    KIND_SIGNAL_BATCH,
};
use crate::error::DaemonError;

mod handlers;
mod storage;

#[cfg(test)]
mod tests;

pub(super) const ORIGIN_PANEL_VERSION: u32 = 1;
pub(super) const ORIGIN_SLOT_ID: SlotId = SlotId::new(813);
pub(super) const ORIGIN_ACTOR: &str = "calyxd-learner-origin";

const STATUS_OK: &str = "200 OK";
const STATUS_CREATED: &str = "201 Created";
const STATUS_BAD_REQUEST: &str = "400 Bad Request";
const STATUS_UNAUTHORIZED: &str = "401 Unauthorized";
const STATUS_FORBIDDEN: &str = "403 Forbidden";
const STATUS_NOT_FOUND: &str = "404 Not Found";
const STATUS_METHOD_NOT_ALLOWED: &str = "405 Method Not Allowed";
const STATUS_CONFLICT: &str = "409 Conflict";
const STATUS_INTERNAL: &str = "500 Internal Server Error";

pub struct OriginResponse {
    pub status: &'static str,
    pub body: String,
}

impl OriginResponse {
    pub(super) fn json(status: &'static str, body: Value) -> Self {
        Self {
            status,
            body: serde_json::to_string(&body).expect("origin response JSON is serializable"),
        }
    }
}

pub struct LearnerOriginService {
    vault: Arc<AsterVault<SystemClock>>,
    secret: String,
    max_body_bytes: usize,
    metrics: Arc<OriginMetrics>,
}

impl LearnerOriginService {
    pub fn from_config(cfg: &LearnerOriginConfig) -> Result<Self, DaemonError> {
        let secret = std::env::var(&cfg.shared_secret_env).map_err(|_| {
            DaemonError::config_invalid(format!(
                "learner_origin shared secret env var `{}` is not set",
                cfg.shared_secret_env
            ))
        })?;
        if secret.is_empty() {
            return Err(DaemonError::config_invalid(format!(
                "learner_origin shared secret env var `{}` is empty",
                cfg.shared_secret_env
            )));
        }
        Self::open(
            cfg.vault_path_resolved(),
            cfg.vault_id,
            cfg.vault_salt.as_bytes().to_vec(),
            secret,
            cfg.max_body_bytes,
        )
    }

    pub fn open(
        vault_path: impl AsRef<Path>,
        vault_id: VaultId,
        vault_salt: Vec<u8>,
        secret: String,
        max_body_bytes: usize,
    ) -> Result<Self, DaemonError> {
        let vault_path = vault_path.as_ref().to_path_buf();
        let vault = AsterVault::open(&vault_path, vault_id, vault_salt, VaultOptions::default())
            .map_err(|error| {
                DaemonError::health_failed(format!(
                    "open learner_origin vault {}: {}",
                    vault_path.display(),
                    error.message
                ))
            })?;
        Ok(Self {
            vault: Arc::new(vault),
            secret,
            max_body_bytes,
            metrics: Arc::new(OriginMetrics::new()),
        })
    }

    pub fn handles_path(&self, path: &str) -> bool {
        route_for_path(path).is_some()
    }

    pub fn max_body_bytes(&self) -> usize {
        self.max_body_bytes
    }

    pub fn metrics(&self) -> Arc<OriginMetrics> {
        Arc::clone(&self.metrics)
    }

    #[cfg(test)]
    pub fn latest_seq(&self) -> u64 {
        self.vault.latest_seq()
    }

    #[cfg(test)]
    pub fn origin_metrics(&self) -> &OriginMetrics {
        &self.metrics
    }

    #[cfg(test)]
    pub fn base_rows(&self) -> Vec<calyx_core::Constellation> {
        self.origin_rows()
            .expect("origin rows scan succeeds")
            .into_iter()
            .map(|row| row.cx)
            .collect()
    }

    pub fn handle(
        &self,
        method: &str,
        path: &str,
        authorization: Option<&str>,
        body: &[u8],
    ) -> OriginResponse {
        let Some(route) = route_for_path(path) else {
            return OriginResponse::json(
                STATUS_NOT_FOUND,
                json!({"error":"CALYX_ORIGIN_ROUTE_NOT_FOUND"}),
            );
        };
        let outcome = if method != "POST" {
            Err(OriginError::new(
                STATUS_METHOD_NOT_ALLOWED,
                "CALYX_ORIGIN_METHOD_NOT_ALLOWED",
                "learner-origin endpoints require POST",
            ))
        } else if !self.authorized(authorization) {
            Err(OriginError::new(
                STATUS_UNAUTHORIZED,
                "CALYX_ORIGIN_UNAUTHORIZED",
                "missing or invalid origin bearer",
            ))
        } else {
            match &route {
                OriginRoute::SignalBatch => self.handle_signal_batch(body),
                OriginRoute::Decision => self.handle_decision(body),
                OriginRoute::Outcome { decision_id } => self.handle_outcome(decision_id, body),
            }
        };
        let response = match outcome {
            Ok(response) => response,
            Err(error) => {
                self.record_rejected(route.kind(), &error);
                OriginResponse::json(
                    error.status,
                    json!({"error": error.code, "message": error.message}),
                )
            }
        };
        self.metrics
            .record_request(route.endpoint(), status_code(response.status));
        response
    }

    fn authorized(&self, authorization: Option<&str>) -> bool {
        let Some(header) = authorization else {
            return false;
        };
        let Some(token) = header.strip_prefix("Bearer ") else {
            return false;
        };
        constant_time_eq(token.as_bytes(), self.secret.as_bytes())
    }

    fn record_rejected(&self, kind: &'static str, error: &OriginError) {
        let result = if error.status == STATUS_INTERNAL {
            "error"
        } else {
            "rejected"
        };
        self.metrics.record_write(kind, result);
    }
}

#[derive(Clone)]
enum OriginRoute {
    SignalBatch,
    Decision,
    Outcome { decision_id: String },
}

impl OriginRoute {
    fn endpoint(&self) -> &'static str {
        match self {
            Self::SignalBatch => ENDPOINT_SIGNALS,
            Self::Decision => ENDPOINT_DECIDE,
            Self::Outcome { .. } => ENDPOINT_OUTCOMES,
        }
    }

    fn kind(&self) -> &'static str {
        match self {
            Self::SignalBatch => KIND_SIGNAL_BATCH,
            Self::Decision => KIND_DECISION,
            Self::Outcome { .. } => KIND_OUTCOME,
        }
    }
}

#[derive(Debug)]
struct OriginError {
    status: &'static str,
    code: &'static str,
    message: String,
}

impl OriginError {
    fn new(status: &'static str, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status,
            code,
            message: message.into(),
        }
    }

    fn bad_request(code: &'static str, message: impl ToString) -> Self {
        Self::new(STATUS_BAD_REQUEST, code, message.to_string())
    }

    fn internal(message: impl Into<String>) -> Self {
        Self::new(STATUS_INTERNAL, "CALYX_ORIGIN_STORAGE_ERROR", message)
    }
}

fn route_for_path(path: &str) -> Option<OriginRoute> {
    if path == "/v1/learner-signals/batches" {
        return Some(OriginRoute::SignalBatch);
    }
    if path == "/v1/interventions/decide" {
        return Some(OriginRoute::Decision);
    }
    let rest = path.strip_prefix("/v1/interventions/")?;
    let decision_id = rest.strip_suffix("/outcomes")?;
    (!decision_id.is_empty()).then(|| OriginRoute::Outcome {
        decision_id: decision_id.to_string(),
    })
}

fn parse_body(body: &[u8]) -> Result<Value, OriginError> {
    serde_json::from_slice(body)
        .map_err(|error| OriginError::bad_request("CALYX_ORIGIN_JSON_INVALID", error))
}

fn ensure_nonempty(field: &str, value: &str) -> Result<(), OriginError> {
    if value.trim().is_empty() {
        return Err(OriginError::bad_request(
            "CALYX_ORIGIN_FIELD_REQUIRED",
            format!("{field} must not be empty"),
        ));
    }
    Ok(())
}

fn base_metadata(kind: &'static str, body_hash: &str) -> BTreeMap<String, String> {
    BTreeMap::from([
        ("origin_kind".to_string(), kind.to_string()),
        ("origin_version".to_string(), "1".to_string()),
        ("payload_sha256".to_string(), body_hash.to_string()),
    ])
}

fn insert_optional(map: &mut BTreeMap<String, String>, key: &str, value: Option<&str>) {
    if let Some(value) = value
        && !value.is_empty()
    {
        map.insert(key.to_string(), value.to_string());
    }
}

fn storage_error(error: calyx_core::CalyxError) -> OriginError {
    OriginError::internal(format!("{}: {}", error.code, error.message))
}

fn sha256_array(bytes: &[u8]) -> [u8; 32] {
    let digest = Sha256::digest(bytes);
    let mut out = [0_u8; 32];
    out.copy_from_slice(&digest);
    out
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex(&sha256_array(bytes))
}

fn stable_id<'a>(prefix: &str, parts: impl IntoIterator<Item = &'a str>) -> String {
    let mut hasher = Sha256::new();
    hasher.update(prefix.as_bytes());
    for part in parts {
        hasher.update((part.len() as u64).to_be_bytes());
        hasher.update(part.as_bytes());
    }
    let digest = hasher.finalize();
    format!("{prefix}-{}", hex(&digest[..16]))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().try_into().unwrap_or(u64::MAX))
        .unwrap_or(0)
}

fn status_code(status: &'static str) -> &'static str {
    status
        .split_once(' ')
        .map(|(code, _)| code)
        .unwrap_or(status)
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right.iter())
        .fold(0_u8, |acc, (left, right)| acc | (left ^ right))
        == 0
}
