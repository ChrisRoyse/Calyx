//! Calyx-controlled forecast-agent launcher.
//!
//! The launcher is intentionally narrow: Calyx/job state supplies exact evidence, output scope, and
//! policy constraints; DeepSeek supplies one JSON forecast; local artifacts and ledger rows are the
//! source of truth after the run.

use std::fs;
use std::path::Path;

use calyx_core::{Clock, CxId, LedgerRef};
use calyx_ledger::{ActorId, EntryKind, LedgerAppender, LedgerCfStore, SubjectId};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::agent_deepseek::{DeepSeekCompletion, call_deepseek};
use crate::{
    AgentForecastArtifactRequest, AgentForecastManifest, AgentSourceSnapshotRef,
    DeepSeekRuntimeSecrets, LocalOnlyPolicy, PolyAction, PolyError, Result, require_policy_allowed,
    write_agent_forecast_artifacts,
};

pub const AGENT_LAUNCH_SCHEMA_VERSION: &str = "poly.agent.launch.v1";
const AGENT_LAUNCH_ACTOR: &str = "calyx-poly-agent-launcher";
const MAX_EVIDENCE_ITEMS: usize = 16;
const MAX_EVIDENCE_BYTES: usize = 32 * 1024;
const MIN_MAX_TOKENS: u32 = 128;
const MAX_MAX_TOKENS: u32 = 4096;
const MIN_TIMEOUT_SECS: u64 = 5;
const MAX_TIMEOUT_SECS: u64 = 120;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentEvidenceSnapshot {
    pub source: AgentSourceSnapshotRef,
    pub title: String,
    pub observed_ts: u64,
    pub expires_ts: u64,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentLauncherRequest {
    pub run_id: String,
    pub created_at: String,
    pub run_ts: u64,
    pub market_id: String,
    pub outcome_id: String,
    pub question: String,
    pub source_snapshot_refs: Vec<AgentSourceSnapshotRef>,
    pub evidence: Vec<AgentEvidenceSnapshot>,
    pub requested_actions: Vec<PolyAction>,
    pub prompt_template_id: String,
    pub prompt_template_version: String,
    pub max_tokens: u32,
    pub timeout_secs: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeepSeekUsage {
    pub prompt_tokens: Option<u64>,
    pub completion_tokens: Option<u64>,
    pub total_tokens: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentLaunchReceipt {
    pub schema_version: String,
    pub run_id: String,
    pub market_id: String,
    pub outcome_id: String,
    pub artifact_dir: String,
    pub policy_ledger_refs: Vec<LedgerRef>,
    pub forecast_ledger_ref: LedgerRef,
    pub manifest: AgentForecastManifest,
    pub provider_response_id: String,
    pub provider_finish_reason: String,
    pub provider_usage: DeepSeekUsage,
}

pub fn launch_deepseek_forecast_agent<S, C>(
    artifact_root: &Path,
    ledger: &mut LedgerAppender<S, C>,
    policy: &LocalOnlyPolicy,
    secrets: &DeepSeekRuntimeSecrets,
    request: &AgentLauncherRequest,
) -> Result<AgentLaunchReceipt>
where
    S: LedgerCfStore,
    C: Clock,
{
    validate_launch_request(request)?;
    let mut policy_ledger_refs = Vec::with_capacity(request.requested_actions.len());
    for action in &request.requested_actions {
        let enforcement = require_policy_allowed(ledger, policy, *action, Some(artifact_root))?;
        policy_ledger_refs.push(enforcement.ledger_ref);
    }

    let rendered_prompt = render_prompt(request);
    let provider = secrets.metadata();
    let response = call_deepseek(secrets, request, &rendered_prompt)?;
    let markdown_prediction = markdown_prediction(request, &response.content_json)?;
    let artifact_request = AgentForecastArtifactRequest {
        run_id: request.run_id.clone(),
        created_at: request.created_at.clone(),
        source_snapshot_refs: request.source_snapshot_refs.clone(),
        prompt_template_id: request.prompt_template_id.clone(),
        prompt_template_version: request.prompt_template_version.clone(),
        rendered_prompt,
        provider,
        raw_response_json: response.content_json.clone(),
        markdown_prediction,
    };

    let manifest = write_agent_forecast_artifacts(artifact_root, &artifact_request)?;
    let final_dir = artifact_root.join(&request.run_id);
    let prepared = match ledger.prepare(
        EntryKind::AgentForecast,
        forecast_subject(request)?,
        forecast_payload(request, &manifest, &response)?,
        ActorId::Service(AGENT_LAUNCH_ACTOR.to_string()),
    ) {
        Ok(prepared) => prepared,
        Err(err) => {
            let cleanup = cleanup_published_artifact(&final_dir);
            return Err(PolyError::agent_launch(
                "POLY_AGENT_LAUNCH_LEDGER_PREPARE_FAILED",
                format!("prepare agent forecast ledger row: {err}; {cleanup}"),
            ));
        }
    };
    let forecast_ledger_ref = match ledger.commit_prepared(&prepared) {
        Ok(ledger_ref) => ledger_ref,
        Err(err) => {
            let cleanup = cleanup_published_artifact(&final_dir);
            return Err(PolyError::agent_launch(
                "POLY_AGENT_LAUNCH_LEDGER_COMMIT_FAILED",
                format!("commit agent forecast ledger row: {err}; {cleanup}"),
            ));
        }
    };

    Ok(AgentLaunchReceipt {
        schema_version: AGENT_LAUNCH_SCHEMA_VERSION.to_string(),
        run_id: request.run_id.clone(),
        market_id: request.market_id.clone(),
        outcome_id: request.outcome_id.clone(),
        artifact_dir: final_dir.display().to_string(),
        policy_ledger_refs,
        forecast_ledger_ref,
        manifest,
        provider_response_id: response.id,
        provider_finish_reason: response.finish_reason,
        provider_usage: response.usage,
    })
}

fn validate_launch_request(request: &AgentLauncherRequest) -> Result<()> {
    validate_id("run_id", &request.run_id)?;
    validate_id("market_id", &request.market_id)?;
    validate_id("outcome_id", &request.outcome_id)?;
    if request.created_at.trim().is_empty() || request.question.trim().is_empty() {
        return Err(PolyError::agent_launch(
            "POLY_AGENT_LAUNCH_EMPTY_TASK_FIELD",
            "created_at and question are required",
        ));
    }
    if request.source_snapshot_refs.is_empty() {
        return Err(PolyError::agent_launch(
            "POLY_AGENT_LAUNCH_EMPTY_SOURCE_SCOPE",
            "at least one source snapshot reference is required",
        ));
    }
    if request.evidence.is_empty() {
        return Err(PolyError::agent_launch(
            "POLY_AGENT_LAUNCH_EMPTY_EVIDENCE",
            "at least one evidence snapshot is required",
        ));
    }
    if request.evidence.len() > MAX_EVIDENCE_ITEMS {
        return Err(PolyError::agent_launch(
            "POLY_AGENT_LAUNCH_TOO_MUCH_EVIDENCE",
            format!("evidence item count exceeds {MAX_EVIDENCE_ITEMS}"),
        ));
    }
    if request.requested_actions.is_empty() {
        return Err(PolyError::agent_launch(
            "POLY_AGENT_LAUNCH_EMPTY_ACTION_SCOPE",
            "requested_actions must include launch and artifact-write scope",
        ));
    }
    if !request
        .requested_actions
        .contains(&PolyAction::LaunchForecastAgent)
    {
        return Err(PolyError::agent_launch(
            "POLY_AGENT_LAUNCH_MISSING_LAUNCH_ACTION",
            "requested_actions must include launch_forecast_agent",
        ));
    }
    if !request
        .requested_actions
        .contains(&PolyAction::WriteForecastArtifact)
    {
        return Err(PolyError::agent_launch(
            "POLY_AGENT_LAUNCH_MISSING_ARTIFACT_ACTION",
            "requested_actions must include write_forecast_artifact",
        ));
    }
    if request.max_tokens < MIN_MAX_TOKENS || request.max_tokens > MAX_MAX_TOKENS {
        return Err(PolyError::agent_launch(
            "POLY_AGENT_LAUNCH_INVALID_MAX_TOKENS",
            format!("max_tokens must be between {MIN_MAX_TOKENS} and {MAX_MAX_TOKENS}"),
        ));
    }
    if request.timeout_secs < MIN_TIMEOUT_SECS || request.timeout_secs > MAX_TIMEOUT_SECS {
        return Err(PolyError::agent_launch(
            "POLY_AGENT_LAUNCH_INVALID_TIMEOUT",
            format!("timeout_secs must be between {MIN_TIMEOUT_SECS} and {MAX_TIMEOUT_SECS}"),
        ));
    }
    for source in &request.source_snapshot_refs {
        validate_id("source cx_id", &source.cx_id)?;
        if source.role.trim().is_empty() {
            return Err(PolyError::agent_launch(
                "POLY_AGENT_LAUNCH_EMPTY_SOURCE_ROLE",
                "source snapshot role must not be empty",
            ));
        }
    }
    for evidence in &request.evidence {
        validate_evidence(request, evidence)?;
    }
    Ok(())
}

fn validate_evidence(
    request: &AgentLauncherRequest,
    evidence: &AgentEvidenceSnapshot,
) -> Result<()> {
    if !request.source_snapshot_refs.contains(&evidence.source) {
        return Err(PolyError::agent_launch(
            "POLY_AGENT_LAUNCH_EVIDENCE_OUT_OF_SCOPE",
            "evidence source was not declared in source_snapshot_refs",
        ));
    }
    if evidence.title.trim().is_empty() || evidence.content.trim().is_empty() {
        return Err(PolyError::agent_launch(
            "POLY_AGENT_LAUNCH_EMPTY_EVIDENCE_FIELD",
            "evidence title and content are required",
        ));
    }
    if evidence.content.len() > MAX_EVIDENCE_BYTES {
        return Err(PolyError::agent_launch(
            "POLY_AGENT_LAUNCH_EVIDENCE_TOO_LARGE",
            format!("one evidence item exceeds {MAX_EVIDENCE_BYTES} bytes"),
        ));
    }
    if evidence.observed_ts > request.run_ts || evidence.expires_ts <= request.run_ts {
        return Err(PolyError::agent_launch(
            "POLY_AGENT_LAUNCH_STALE_EVIDENCE",
            "evidence must be observed no later than run_ts and expire after run_ts",
        ));
    }
    Ok(())
}

fn validate_id(field: &str, value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 64
        || !value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
    {
        return Err(PolyError::agent_launch(
            "POLY_AGENT_LAUNCH_INVALID_ID",
            format!("{field} must be 1..=64 ASCII letters, numbers, hyphen, or underscore"),
        ));
    }
    Ok(())
}

fn render_prompt(request: &AgentLauncherRequest) -> String {
    let evidence = request
        .evidence
        .iter()
        .enumerate()
        .map(|(idx, item)| {
            format!(
                "## Evidence {idx}: {}\nsource_cx_id: {}\nrole: {}\nobserved_ts: {}\nexpires_ts: {}\n{}\n",
                item.title,
                item.source.cx_id,
                item.source.role,
                item.observed_ts,
                item.expires_ts,
                item.content
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "# Poly Forecast Agent Task\n\
         schema_version: {AGENT_LAUNCH_SCHEMA_VERSION}\n\
         run_id: {}\n\
         market_id: {}\n\
         outcome_id: {}\n\
         question: {}\n\n\
         You are controlled by Calyx. Return exactly one JSON object and no markdown. \
         The object must match this schema exactly:\n\
         {{\"probability\":0.64,\"confidence\":0.74,\"rationale\":\"string\",\
         \"constraints\":[\"local-only forecast artifact\",\"no trading action\"],\
         \"no_trade_policy_assertion\":true}}\n\
         `probability` must be a JSON number in [0,1]; `confidence` must be a JSON number in \
         [0,1) (strictly less than 1 — grounded confidence never reaches certainty). \
         `rationale` must be a non-empty JSON string. \
         `constraints` must be a JSON array of strings, not a string. \
         `no_trade_policy_assertion` must be the JSON boolean true. \
         The JSON values must be grounded only in the evidence below. Do not include \
         tool calls, browsing requests, order signing, order submission, bankroll management, \
         or trading instructions.\n\n{}",
        request.run_id, request.market_id, request.outcome_id, request.question, evidence
    )
}

fn markdown_prediction(request: &AgentLauncherRequest, content_json: &str) -> Result<String> {
    let value: Value = serde_json::from_str(content_json).map_err(|err| {
        PolyError::agent_launch(
            "POLY_AGENT_LAUNCH_RESPONSE_JSON_INVALID",
            format!("decode DeepSeek JSON content before markdown: {err}"),
        )
    })?;
    let probability = value
        .get("probability")
        .and_then(Value::as_f64)
        .ok_or_else(|| {
            PolyError::agent_launch(
                "POLY_AGENT_LAUNCH_RESPONSE_PROBABILITY_MISSING",
                "response content must include probability",
            )
        })?;
    let confidence = value
        .get("confidence")
        .and_then(Value::as_f64)
        .ok_or_else(|| {
            PolyError::agent_launch(
                "POLY_AGENT_LAUNCH_RESPONSE_CONFIDENCE_MISSING",
                "response content must include confidence",
            )
        })?;
    let rationale = value
        .get("rationale")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            PolyError::agent_launch(
                "POLY_AGENT_LAUNCH_RESPONSE_RATIONALE_MISSING",
                "response content must include rationale",
            )
        })?;
    Ok(format!(
        "# Poly Forecast\n\nRun: {}\nMarket: {}\nOutcome: {}\nProbability: {:.6}\nConfidence: {:.6}\n\n## Rationale\n{}\n\n## Policy\nLocal forecast artifact only. No order signing, order submission, or bankroll management.\n",
        request.run_id, request.market_id, request.outcome_id, probability, confidence, rationale
    ))
}

fn forecast_subject(request: &AgentLauncherRequest) -> Result<SubjectId> {
    let cx_id = request.source_snapshot_refs[0]
        .cx_id
        .parse::<CxId>()
        .map_err(|err| {
            PolyError::agent_launch(
                "POLY_AGENT_LAUNCH_SOURCE_CX_ID_INVALID",
                format!("source cx_id did not parse as CxId: {err}"),
            )
        })?;
    Ok(SubjectId::Cx(cx_id))
}

fn forecast_payload(
    request: &AgentLauncherRequest,
    manifest: &AgentForecastManifest,
    response: &DeepSeekCompletion,
) -> Result<Vec<u8>> {
    let payload = json!({
        "schema_version": AGENT_LAUNCH_SCHEMA_VERSION,
        "market_id": request.market_id,
        "outcome_id": request.outcome_id,
        "provider_response_id": response.id,
        "provider_finish_reason": response.finish_reason,
        "provider_usage": response.usage,
        "manifest": manifest.provenance_payload()
    });
    serde_json::to_vec(&payload).map_err(|err| {
        PolyError::agent_launch(
            "POLY_AGENT_LAUNCH_LEDGER_PAYLOAD_ENCODE_FAILED",
            err.to_string(),
        )
    })
}

fn cleanup_published_artifact(final_dir: &Path) -> String {
    fs::remove_dir_all(final_dir)
        .map(|()| "published agent artifact cleanup succeeded".to_string())
        .unwrap_or_else(|err| {
            format!(
                "published agent artifact cleanup failed for {}: {err}",
                final_dir.display()
            )
        })
}
