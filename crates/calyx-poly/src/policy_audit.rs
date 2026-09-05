//! Durable audit helpers for the local-only runtime policy.

use std::fs;
use std::path::{Component, Path, PathBuf};

use calyx_core::{Clock, LedgerRef};
use calyx_ledger::{ActorId, EntryKind, LedgerAppender, LedgerCfStore, SubjectId};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{LocalOnlyPolicy, PolicyDecision, PolyAction, PolyError, Result};

pub const POLICY_AUDIT_SCHEMA_VERSION: &str = "poly.policy.audit.v1";
const POLICY_ACTOR: &str = "calyx-poly-policy";

/// One persisted runtime-policy enforcement event.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PolicyEnforcement {
    pub schema_version: String,
    pub action: String,
    pub decision: PolicyDecision,
    pub policy: LocalOnlyPolicy,
    pub requested_artifact_path: Option<String>,
    pub ledger_ref: LedgerRef,
}

/// JSON snapshot of the active local-only policy contract.
pub fn policy_config_snapshot(policy: &LocalOnlyPolicy) -> Value {
    json!({
        "schema_version": POLICY_AUDIT_SCHEMA_VERSION,
        "policy": policy,
        "allowed_actions": [
            PolyAction::ReadPublicData.as_str(),
            PolyAction::IngestSnapshot.as_str(),
            PolyAction::UpdateAssociations.as_str(),
            PolyAction::WriteForecastArtifact.as_str(),
            PolyAction::AdmitForecast.as_str(),
            PolyAction::ScoreForecast.as_str(),
            PolyAction::LaunchForecastAgent.as_str(),
            PolyAction::RunScheduler.as_str()
        ],
        "forbidden_trading_actions": PolyAction::FORBIDDEN_TRADING_ACTIONS
            .iter()
            .map(|action| action.as_str())
            .collect::<Vec<_>>()
    })
}

/// Writes a local policy config snapshot for source-of-truth readback.
pub fn write_policy_config_snapshot(root: &Path, policy: &LocalOnlyPolicy) -> Result<PathBuf> {
    fs::create_dir_all(root).map_err(|err| {
        PolyError::policy("CALYX_POLY_POLICY_CONFIG_WRITE_FAILED", err.to_string())
    })?;
    let path = root.join("policy-config.json");
    let bytes = serde_json::to_vec_pretty(&policy_config_snapshot(policy)).map_err(|err| {
        PolyError::policy("CALYX_POLY_POLICY_CONFIG_ENCODE_FAILED", err.to_string())
    })?;
    fs::write(&path, bytes).map_err(|err| {
        PolyError::policy("CALYX_POLY_POLICY_CONFIG_WRITE_FAILED", err.to_string())
    })?;
    Ok(path)
}

/// Records a policy decision in the Calyx ledger without executing the action.
pub fn record_policy_decision<S, C>(
    ledger: &mut LedgerAppender<S, C>,
    policy: &LocalOnlyPolicy,
    action: PolyAction,
    requested_artifact_path: Option<&Path>,
) -> Result<PolicyEnforcement>
where
    S: LedgerCfStore,
    C: Clock,
{
    let decision = policy.enforce(action);
    let requested_artifact_path = requested_artifact_path.map(|path| path.display().to_string());
    let payload = policy_payload(
        policy,
        action,
        &decision,
        requested_artifact_path.as_deref(),
    )?;
    let ledger_ref = ledger
        .append(
            EntryKind::Policy,
            policy_subject(action),
            payload,
            ActorId::Service(POLICY_ACTOR.to_string()),
        )
        .map_err(|err| {
            PolyError::policy(
                "CALYX_POLY_POLICY_LEDGER_APPEND_FAILED",
                format!("append policy decision ledger row: {err}"),
            )
        })?;
    Ok(PolicyEnforcement {
        schema_version: POLICY_AUDIT_SCHEMA_VERSION.to_string(),
        action: action.as_str().to_string(),
        decision,
        policy: policy.clone(),
        requested_artifact_path,
        ledger_ref,
    })
}

/// Records the policy decision and fails closed unless the action is explicitly allowed.
pub fn require_policy_allowed<S, C>(
    ledger: &mut LedgerAppender<S, C>,
    policy: &LocalOnlyPolicy,
    action: PolyAction,
    requested_artifact_path: Option<&Path>,
) -> Result<PolicyEnforcement>
where
    S: LedgerCfStore,
    C: Clock,
{
    let enforcement = record_policy_decision(ledger, policy, action, requested_artifact_path)?;
    if enforcement.decision.allowed {
        return Ok(enforcement);
    }
    Err(PolyError::policy(
        enforcement.decision.code.clone(),
        enforcement.decision.reason.clone(),
    ))
}

/// Writes a local artifact only after a persisted allow decision.
pub fn write_policy_guarded_artifact<S, C>(
    artifact_root: &Path,
    ledger: &mut LedgerAppender<S, C>,
    policy: &LocalOnlyPolicy,
    action: PolyAction,
    relative_artifact_path: &Path,
    bytes: &[u8],
) -> Result<PolicyEnforcement>
where
    S: LedgerCfStore,
    C: Clock,
{
    let artifact_path = safe_artifact_path(artifact_root, relative_artifact_path)?;
    let enforcement = require_policy_allowed(ledger, policy, action, Some(&artifact_path))?;
    if let Some(parent) = artifact_path.parent() {
        fs::create_dir_all(parent).map_err(|err| {
            PolyError::policy("CALYX_POLY_POLICY_ARTIFACT_WRITE_FAILED", err.to_string())
        })?;
    }
    fs::write(&artifact_path, bytes).map_err(|err| {
        PolyError::policy("CALYX_POLY_POLICY_ARTIFACT_WRITE_FAILED", err.to_string())
    })?;
    Ok(enforcement)
}

fn policy_payload(
    policy: &LocalOnlyPolicy,
    action: PolyAction,
    decision: &PolicyDecision,
    requested_artifact_path: Option<&str>,
) -> Result<Vec<u8>> {
    let requested_artifact = requested_artifact_path.map(|path| {
        let path_hash = blake3::hash(path.as_bytes()).to_hex().to_string();
        let file_name = Path::new(path)
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("artifact")
            .to_string();
        json!({
            "file": file_name,
            "path_hash": path_hash
        })
    });
    let code_parts: Vec<&str> = decision.code.split('_').collect();
    let code_hash = blake3::hash(decision.code.as_bytes()).to_hex().to_string();
    serde_json::to_vec(&json!({
        "schema_version": POLICY_AUDIT_SCHEMA_VERSION,
        "action": action.as_str(),
        "allowed": decision.allowed,
        "code_parts": code_parts,
        "code_hash": code_hash,
        "reason": decision.reason,
        "policy": policy,
        "requested_artifact": requested_artifact
    }))
    .map_err(|err| {
        PolyError::policy(
            "CALYX_POLY_POLICY_PAYLOAD_ENCODE_FAILED",
            format!("encode policy payload: {err}"),
        )
    })
}

fn policy_subject(action: PolyAction) -> SubjectId {
    let digest = blake3::hash(format!("poly-policy:{}", action.as_str()).as_bytes());
    SubjectId::Query(digest.as_bytes().to_vec())
}

fn safe_artifact_path(root: &Path, relative: &Path) -> Result<PathBuf> {
    if relative.as_os_str().is_empty() || relative.is_absolute() {
        return Err(PolyError::policy(
            "CALYX_POLY_POLICY_INVALID_ARTIFACT_PATH",
            "artifact path must be a non-empty relative path",
        ));
    }
    if relative.components().any(|component| {
        matches!(
            component,
            Component::ParentDir | Component::Prefix(_) | Component::RootDir
        )
    }) {
        return Err(PolyError::policy(
            "CALYX_POLY_POLICY_INVALID_ARTIFACT_PATH",
            "artifact path must stay inside the artifact root",
        ));
    }
    Ok(root.join(relative))
}
