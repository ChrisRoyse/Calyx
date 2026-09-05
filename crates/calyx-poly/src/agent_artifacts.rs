//! Local source-of-truth artifacts for DeepSeek forecast-agent runs.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{DeepSeekSecretMetadata, PolyError, Result};

pub const AGENT_FORECAST_SCHEMA_VERSION: &str = "poly.agent.forecast.v1";

/// One local Calyx source snapshot supplied to the forecast agent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentSourceSnapshotRef {
    pub cx_id: String,
    pub role: String,
    pub snapshot: u64,
}

/// Prompt template and rendered prompt provenance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentPromptArtifact {
    pub template_id: String,
    pub template_version: String,
    pub rendered_prompt_path: String,
    pub rendered_prompt_blake3: String,
}

/// Raw provider response provenance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentResponseArtifact {
    pub raw_response_path: String,
    pub raw_response_blake3: String,
}

/// Parsed forecast extracted from the provider response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentParsedForecast {
    pub probability: f64,
    pub confidence: f64,
    pub rationale_path: String,
    pub rationale_blake3: String,
    pub constraints: Vec<String>,
    pub no_trade_policy_assertion: bool,
}

/// Durable manifest tying every local artifact together.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentForecastManifest {
    pub schema_version: String,
    pub run_id: String,
    pub created_at: String,
    pub source_snapshot_refs: Vec<AgentSourceSnapshotRef>,
    pub prompt: AgentPromptArtifact,
    pub provider: DeepSeekSecretMetadata,
    pub response: AgentResponseArtifact,
    pub parsed_forecast_path: String,
    pub parsed_forecast_blake3: String,
    pub parsed_forecast: AgentParsedForecast,
    pub markdown_prediction_path: String,
    pub markdown_prediction_blake3: String,
    pub no_trade_policy_assertion: bool,
}

impl AgentForecastManifest {
    /// Hash/id-only ledger payload accepted by the Calyx ledger secret guard.
    pub fn provenance_payload(&self) -> Value {
        let source_cx_id = self
            .source_snapshot_refs
            .first()
            .map(|source| source.cx_id.clone())
            .unwrap_or_default();
        json!({
            "schema_version": self.schema_version,
            "run_id": self.run_id,
            "source_cx_id": source_cx_id,
            "source_count": self.source_snapshot_refs.len(),
            "prompt_template_id": self.prompt.template_id,
            "prompt_template_version": self.prompt.template_version,
            "rendered_prompt_hash": self.prompt.rendered_prompt_blake3,
            "raw_response_hash": self.response.raw_response_blake3,
            "parsed_forecast_hash": self.parsed_forecast_blake3,
            "rationale_hash": self.parsed_forecast.rationale_blake3,
            "markdown_prediction_hash": self.markdown_prediction_blake3,
            "provider_model": self.provider.model,
            "probability": self.parsed_forecast.probability,
            "confidence": self.parsed_forecast.confidence,
            "no_trade_policy_assertion": self.no_trade_policy_assertion
        })
    }
}

/// Inputs required to create one forecast-agent artifact bundle.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentForecastArtifactRequest {
    pub run_id: String,
    pub created_at: String,
    pub source_snapshot_refs: Vec<AgentSourceSnapshotRef>,
    pub prompt_template_id: String,
    pub prompt_template_version: String,
    pub rendered_prompt: String,
    pub provider: DeepSeekSecretMetadata,
    pub raw_response_json: String,
    pub markdown_prediction: String,
}

/// Validate and write a complete forecast-agent artifact bundle.
pub fn write_agent_forecast_artifacts(
    root: &Path,
    request: &AgentForecastArtifactRequest,
) -> Result<AgentForecastManifest> {
    validate_request_shape(request)?;
    let parsed_response = parse_agent_response(&request.raw_response_json)?;
    reject_forbidden_trading_text(&parsed_response.rationale)?;
    reject_forbidden_trading_text(&request.markdown_prediction)?;

    let final_dir = root.join(&request.run_id);
    let staging_dir = root.join(format!(".{}.tmp", request.run_id));
    if final_dir.exists() {
        return Err(PolyError::agent_artifact(
            "POLY_AGENT_ARTIFACT_RUN_EXISTS",
            format!(
                "artifact run directory already exists: {}",
                final_dir.display()
            ),
        ));
    }
    if staging_dir.exists() {
        return Err(PolyError::agent_artifact(
            "POLY_AGENT_ARTIFACT_STAGING_EXISTS",
            format!(
                "artifact staging directory already exists: {}",
                staging_dir.display()
            ),
        ));
    }

    fs::create_dir_all(staging_dir.join("prompt")).map_err(|err| {
        PolyError::agent_artifact("POLY_AGENT_ARTIFACT_WRITE_FAILED", err.to_string())
    })?;
    fs::create_dir_all(staging_dir.join("response")).map_err(|err| {
        PolyError::agent_artifact("POLY_AGENT_ARTIFACT_WRITE_FAILED", err.to_string())
    })?;
    fs::create_dir_all(staging_dir.join("forecast")).map_err(|err| {
        PolyError::agent_artifact("POLY_AGENT_ARTIFACT_WRITE_FAILED", err.to_string())
    })?;

    let prompt_path = staging_dir.join("prompt").join("rendered-prompt.md");
    let response_path = staging_dir.join("response").join("raw-response.json");
    let rationale_path = staging_dir.join("forecast").join("rationale.md");
    let parsed_path = staging_dir.join("forecast").join("parsed-forecast.json");
    let markdown_path = staging_dir.join("prediction.md");

    write_bytes(&prompt_path, request.rendered_prompt.as_bytes())?;
    write_bytes(&response_path, request.raw_response_json.as_bytes())?;
    write_bytes(&rationale_path, parsed_response.rationale.as_bytes())?;
    write_json_file(&parsed_path, &parsed_response)?;
    write_bytes(&markdown_path, request.markdown_prediction.as_bytes())?;

    let manifest = AgentForecastManifest {
        schema_version: AGENT_FORECAST_SCHEMA_VERSION.to_string(),
        run_id: request.run_id.clone(),
        created_at: request.created_at.clone(),
        source_snapshot_refs: request.source_snapshot_refs.clone(),
        prompt: AgentPromptArtifact {
            template_id: request.prompt_template_id.clone(),
            template_version: request.prompt_template_version.clone(),
            rendered_prompt_path: rel_path("prompt/rendered-prompt.md"),
            rendered_prompt_blake3: hash_file(&prompt_path)?,
        },
        provider: request.provider.clone(),
        response: AgentResponseArtifact {
            raw_response_path: rel_path("response/raw-response.json"),
            raw_response_blake3: hash_file(&response_path)?,
        },
        parsed_forecast_path: rel_path("forecast/parsed-forecast.json"),
        parsed_forecast_blake3: hash_file(&parsed_path)?,
        parsed_forecast: AgentParsedForecast {
            probability: parsed_response.probability,
            confidence: parsed_response.confidence,
            rationale_path: rel_path("forecast/rationale.md"),
            rationale_blake3: hash_file(&rationale_path)?,
            constraints: parsed_response.constraints,
            no_trade_policy_assertion: parsed_response.no_trade_policy_assertion,
        },
        markdown_prediction_path: rel_path("prediction.md"),
        markdown_prediction_blake3: hash_file(&markdown_path)?,
        no_trade_policy_assertion: parsed_response.no_trade_policy_assertion,
    };

    write_json_file(&staging_dir.join("manifest.json"), &manifest)?;
    fs::rename(&staging_dir, &final_dir).map_err(|err| {
        PolyError::agent_artifact("POLY_AGENT_ARTIFACT_PUBLISH_FAILED", err.to_string())
    })?;
    Ok(manifest)
}

fn validate_request_shape(request: &AgentForecastArtifactRequest) -> Result<()> {
    if request.run_id.is_empty()
        || !request
            .run_id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
    {
        return Err(PolyError::agent_artifact(
            "POLY_AGENT_ARTIFACT_INVALID_RUN_ID",
            "run_id must contain only ASCII letters, numbers, hyphen, or underscore",
        ));
    }
    if request.source_snapshot_refs.is_empty() {
        return Err(PolyError::agent_artifact(
            "POLY_AGENT_ARTIFACT_MISSING_SOURCES",
            "at least one source snapshot reference is required",
        ));
    }
    if request.prompt_template_id.is_empty() || request.prompt_template_version.is_empty() {
        return Err(PolyError::agent_artifact(
            "POLY_AGENT_ARTIFACT_MISSING_PROMPT_TEMPLATE",
            "prompt template id and version are required",
        ));
    }
    if request.rendered_prompt.is_empty() {
        return Err(PolyError::agent_artifact(
            "POLY_AGENT_ARTIFACT_EMPTY_PROMPT",
            "rendered prompt must not be empty",
        ));
    }
    if request.markdown_prediction.is_empty() {
        return Err(PolyError::agent_artifact(
            "POLY_AGENT_ARTIFACT_EMPTY_MARKDOWN",
            "markdown prediction must not be empty",
        ));
    }
    Ok(())
}

#[derive(Debug, Serialize)]
struct ParsedResponseForFile {
    probability: f64,
    confidence: f64,
    rationale: String,
    constraints: Vec<String>,
    no_trade_policy_assertion: bool,
}

fn parse_agent_response(raw: &str) -> Result<ParsedResponseForFile> {
    let value: Value = serde_json::from_str(raw).map_err(|err| {
        PolyError::agent_artifact("POLY_AGENT_RESPONSE_INVALID_JSON", err.to_string())
    })?;
    let probability = required_unit_f64(&value, "probability")?;
    let confidence = required_confidence_f64(&value)?;
    let rationale = value
        .get("rationale")
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
        .ok_or_else(|| {
            PolyError::agent_artifact(
                "POLY_AGENT_RESPONSE_MISSING_RATIONALE",
                "rationale must be a non-empty string",
            )
        })?
        .to_string();
    let constraints = value
        .get("constraints")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            PolyError::agent_artifact(
                "POLY_AGENT_RESPONSE_MISSING_CONSTRAINTS",
                "constraints must be an array of strings",
            )
        })?
        .iter()
        .map(|item| {
            item.as_str().map(str::to_string).ok_or_else(|| {
                PolyError::agent_artifact(
                    "POLY_AGENT_RESPONSE_INVALID_CONSTRAINT",
                    "each constraint must be a string",
                )
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let no_trade_policy_assertion = value
        .get("no_trade_policy_assertion")
        .and_then(Value::as_bool)
        .ok_or_else(|| {
            PolyError::agent_artifact(
                "POLY_AGENT_RESPONSE_MISSING_NO_TRADE_ASSERTION",
                "no_trade_policy_assertion must be true",
            )
        })?;
    if !no_trade_policy_assertion {
        return Err(PolyError::agent_artifact(
            "POLY_AGENT_RESPONSE_NO_TRADE_ASSERTION_FALSE",
            "no_trade_policy_assertion must be true",
        ));
    }

    Ok(ParsedResponseForFile {
        probability,
        confidence,
        rationale,
        constraints,
        no_trade_policy_assertion,
    })
}

fn required_unit_f64(value: &Value, field: &str) -> Result<f64> {
    let number = value.get(field).and_then(Value::as_f64).ok_or_else(|| {
        PolyError::agent_artifact(
            format!("POLY_AGENT_RESPONSE_MISSING_{}", field.to_ascii_uppercase()),
            format!("{field} must be a finite number in [0, 1]"),
        )
    })?;
    if !number.is_finite() || !(0.0..=1.0).contains(&number) {
        return Err(PolyError::agent_artifact(
            format!(
                "POLY_AGENT_RESPONSE_{}_OUT_OF_RANGE",
                field.to_ascii_uppercase()
            ),
            format!("{field} must be finite and in [0, 1]"),
        ));
    }
    Ok(number)
}

/// Validates an agent-supplied **confidence** against the handbook ceiling. Grounded confidence is
/// `n/(n+1)` and the forecast confidence ceiling is `min(raw, self-consistency, DPI)` — both
/// strictly below 1. A single ungrounded LLM call claiming `confidence: 1.0` (perfect certainty)
/// must never be persisted as a trusted artifact, so confidence is required to be finite in
/// `[0, 1)`. (See #87 / #184; probability keeps the inclusive `[0, 1]` unit check.)
fn required_confidence_f64(value: &Value) -> Result<f64> {
    let number = value
        .get("confidence")
        .and_then(Value::as_f64)
        .ok_or_else(|| {
            PolyError::agent_artifact(
                "POLY_AGENT_RESPONSE_MISSING_CONFIDENCE",
                "confidence must be a finite number in [0, 1)",
            )
        })?;
    if !number.is_finite() || !(0.0..1.0).contains(&number) {
        return Err(PolyError::agent_artifact(
            "POLY_AGENT_RESPONSE_CONFIDENCE_CEILING",
            format!(
                "confidence must be finite and in [0, 1); grounded confidence is n/(n+1) and never \
                 reaches 1 (got {number}); an ungrounded single-call certainty of 1.0 is not admissible"
            ),
        ));
    }
    Ok(number)
}

fn reject_forbidden_trading_text(text: &str) -> Result<()> {
    let lower = text.to_ascii_lowercase();
    let forbidden = [
        "place a bet",
        "place bets",
        "sign order",
        "submit order",
        "cancel order",
        "manage bankroll",
        "execute trade",
        "order placement",
        "use polymarket to trade",
    ];
    if let Some(hit) = forbidden.iter().find(|phrase| lower.contains(**phrase)) {
        return Err(PolyError::agent_artifact(
            "POLY_AGENT_RESPONSE_FORBIDDEN_TRADING_INSTRUCTION",
            format!("forbidden trading instruction detected: {hit}"),
        ));
    }
    Ok(())
}

fn write_bytes(path: &Path, bytes: &[u8]) -> Result<()> {
    fs::write(path, bytes).map_err(|err| {
        PolyError::agent_artifact("POLY_AGENT_ARTIFACT_WRITE_FAILED", err.to_string())
    })
}

fn write_json_file<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(value).map_err(|err| {
        PolyError::agent_artifact("POLY_AGENT_ARTIFACT_ENCODE_FAILED", err.to_string())
    })?;
    write_bytes(path, &bytes)
}

fn hash_file(path: &Path) -> Result<String> {
    let bytes = fs::read(path).map_err(|err| {
        PolyError::agent_artifact("POLY_AGENT_ARTIFACT_READBACK_FAILED", err.to_string())
    })?;
    Ok(blake3::hash(&bytes).to_hex().to_string())
}

fn rel_path(path: &str) -> String {
    PathBuf::from(path).display().to_string()
}
