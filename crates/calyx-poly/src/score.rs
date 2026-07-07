//! Forecast outcome scoring with durable local artifacts and ledger provenance.

use std::fs;
use std::path::{Path, PathBuf};

use calyx_core::{Clock, LedgerRef};
use calyx_ledger::{ActorId, EntryKind, LedgerAppender, LedgerCfStore, SubjectId};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::pending_forecast_payload::safe_ref;
use crate::{PolyError, Result};

pub const FORECAST_SCORE_SCHEMA_VERSION: &str = "poly.forecast.score.v1";
const SCORE_ACTOR: &str = "calyx-poly-score";

/// Source component that produced the forecast being scored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ForecastSource {
    CalyxNative,
    DeepSeekAgent,
    BaselineMarket,
}

impl ForecastSource {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CalyxNative => "calyx_native",
            Self::DeepSeekAgent => "deepseek_agent",
            Self::BaselineMarket => "baseline_market",
        }
    }
}

/// Resolved binary outcome used as the scoring truth source.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResolvedOutcome {
    pub outcome_id: String,
    pub resolved: bool,
    pub actual_win: bool,
    pub resolved_ts: u64,
    pub source: String,
    pub version: u32,
}

/// Request to score one stored forecast against one resolved outcome.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ForecastScoreRequest {
    pub score_id: String,
    pub forecast_id: String,
    pub forecast_version: u32,
    pub current_forecast_version: u32,
    pub market_id: String,
    pub outcome_id: String,
    pub source: ForecastSource,
    pub provider: Option<String>,
    pub probability: f64,
    pub confidence: f64,
    pub forecast_ts: u64,
    pub scored_ts: u64,
    pub horizon_secs: u64,
    pub sufficiency_state: String,
    pub previous_probability: Option<f64>,
    pub forecast_artifact_hash: String,
    pub outcome: ResolvedOutcome,
    pub calibration_bin_count: u32,
}

/// Probability calibration bin containing this forecast.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CalibrationBin {
    pub index: u32,
    pub count: u32,
    pub lower_inclusive: f64,
    pub upper_exclusive: f64,
}

/// Per-forecast metrics persisted in `score.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ForecastScoreMetrics {
    pub brier: f64,
    pub log_loss: Option<f64>,
    pub log_loss_defined: bool,
    pub log_loss_undefined_reason: Option<String>,
    pub direction_accuracy: bool,
    pub calibration_bin: CalibrationBin,
    pub probability_drift: Option<f64>,
    pub absolute_probability_drift: Option<f64>,
}

/// Manifest tying the local forecast/outcome/score artifacts to the score ledger row.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ForecastScoreManifest {
    pub schema_version: String,
    pub score_id: String,
    pub forecast_id: String,
    pub forecast_version: u32,
    pub outcome_id: String,
    pub source: ForecastSource,
    pub provider: Option<String>,
    pub sufficiency_state: String,
    pub forecast_path: String,
    pub forecast_hash: String,
    pub outcome_path: String,
    pub outcome_hash: String,
    pub score_path: String,
    pub score_hash: String,
    pub metrics: ForecastScoreMetrics,
    pub ledger_ref: LedgerRef,
}

/// Validate, score, persist, and ledger-stamp one forecast outcome score.
pub fn write_forecast_score_artifacts<S, C>(
    root: &Path,
    ledger: &mut LedgerAppender<S, C>,
    request: &ForecastScoreRequest,
) -> Result<ForecastScoreManifest>
where
    S: LedgerCfStore,
    C: Clock,
{
    validate_request(request)?;
    let final_dir = root.join(&request.score_id);
    let staging_dir = root.join(format!(".{}.tmp", request.score_id));
    if final_dir.exists() {
        return Err(PolyError::score(
            "CALYX_POLY_SCORE_DUPLICATE",
            format!("score artifact already exists: {}", final_dir.display()),
        ));
    }
    if staging_dir.exists() {
        return Err(PolyError::score(
            "CALYX_POLY_SCORE_STAGING_EXISTS",
            format!(
                "score staging directory already exists: {}",
                staging_dir.display()
            ),
        ));
    }

    fs::create_dir_all(&staging_dir).map_err(|err| {
        PolyError::score("CALYX_POLY_SCORE_ARTIFACT_WRITE_FAILED", err.to_string())
    })?;
    let forecast_path = staging_dir.join("forecast.json");
    let outcome_path = staging_dir.join("outcome.json");
    let score_path = staging_dir.join("score.json");
    let manifest_path = staging_dir.join("manifest.json");

    let metrics = score_metrics(request)?;
    write_json_file(&forecast_path, &forecast_artifact(request))?;
    write_json_file(&outcome_path, &request.outcome)?;
    write_json_file(&score_path, &metrics)?;

    let forecast_hash = hash_file(&forecast_path)?;
    let outcome_hash = hash_file(&outcome_path)?;
    let score_hash = hash_file(&score_path)?;
    let prepared = match ledger.prepare(
        EntryKind::Score,
        score_subject(request),
        score_payload(
            request,
            &metrics,
            &forecast_hash,
            &outcome_hash,
            &score_hash,
        )?,
        ActorId::Service(SCORE_ACTOR.to_string()),
    ) {
        Ok(value) => value,
        Err(err) => {
            let _ = fs::remove_dir_all(&staging_dir);
            return Err(PolyError::score(
                "CALYX_POLY_SCORE_LEDGER_PREPARE_FAILED",
                format!("prepare forecast score ledger row: {err}"),
            ));
        }
    };
    let ledger_ref = prepared.ledger_ref();

    let manifest = ForecastScoreManifest {
        schema_version: FORECAST_SCORE_SCHEMA_VERSION.to_string(),
        score_id: request.score_id.clone(),
        forecast_id: request.forecast_id.clone(),
        forecast_version: request.forecast_version,
        outcome_id: request.outcome_id.clone(),
        source: request.source,
        provider: request.provider.clone(),
        sufficiency_state: request.sufficiency_state.clone(),
        forecast_path: rel_path("forecast.json"),
        forecast_hash,
        outcome_path: rel_path("outcome.json"),
        outcome_hash,
        score_path: rel_path("score.json"),
        score_hash,
        metrics,
        ledger_ref,
    };
    write_json_file(&manifest_path, &manifest)?;
    fs::rename(&staging_dir, &final_dir).map_err(|err| {
        PolyError::score("CALYX_POLY_SCORE_ARTIFACT_PUBLISH_FAILED", err.to_string())
    })?;
    if let Err(err) = ledger.commit_prepared(&prepared) {
        let cleanup = fs::remove_dir_all(&final_dir)
            .map(|()| "published score artifact cleanup succeeded".to_string())
            .unwrap_or_else(|cleanup_err| {
                format!(
                    "published score artifact cleanup failed for {}: {cleanup_err}",
                    final_dir.display()
                )
            });
        return Err(PolyError::score(
            "CALYX_POLY_SCORE_LEDGER_COMMIT_FAILED",
            format!("commit forecast score ledger row: {err}; {cleanup}"),
        ));
    }
    Ok(manifest)
}

fn validate_request(request: &ForecastScoreRequest) -> Result<()> {
    validate_score_id("score_id", &request.score_id)?;
    validate_public_id("forecast_id", &request.forecast_id)?;
    validate_public_id("market_id", &request.market_id)?;
    validate_public_id("outcome_id", &request.outcome_id)?;
    validate_public_id("outcome.outcome_id", &request.outcome.outcome_id)?;
    if request.outcome_id != request.outcome.outcome_id {
        return Err(PolyError::score(
            "CALYX_POLY_SCORE_OUTCOME_ID_MISMATCH",
            "request outcome_id does not match resolved outcome id",
        ));
    }
    if request.forecast_version == 0 || request.current_forecast_version == 0 {
        return Err(PolyError::score(
            "CALYX_POLY_SCORE_INVALID_VERSION",
            "forecast versions must be positive",
        ));
    }
    if request.forecast_version != request.current_forecast_version {
        return Err(PolyError::score(
            "CALYX_POLY_SCORE_STALE_FORECAST_VERSION",
            "forecast_version is not the current stored forecast version",
        ));
    }
    if !request.outcome.resolved {
        return Err(PolyError::score(
            "CALYX_POLY_SCORE_UNRESOLVED_OUTCOME",
            "cannot score a forecast before the outcome is resolved",
        ));
    }
    validate_unit(request.probability, "probability")?;
    validate_confidence(request.confidence)?;
    if let Some(previous) = request.previous_probability {
        validate_unit(previous, "previous_probability")?;
    }
    if request.calibration_bin_count == 0 || request.calibration_bin_count > 100 {
        return Err(PolyError::score(
            "CALYX_POLY_SCORE_INVALID_BIN_COUNT",
            "calibration_bin_count must be in 1..=100",
        ));
    }
    validate_label("provider", request.provider.as_deref())?;
    validate_label("sufficiency_state", Some(&request.sufficiency_state))?;
    validate_hash("forecast_artifact_hash", &request.forecast_artifact_hash)
}

fn score_metrics(request: &ForecastScoreRequest) -> Result<ForecastScoreMetrics> {
    let y = if request.outcome.actual_win { 1.0 } else { 0.0 };
    let err = request.probability - y;
    let log_loss = if request.probability > 0.0 && request.probability < 1.0 {
        Some(if request.outcome.actual_win {
            -request.probability.ln()
        } else {
            -(1.0 - request.probability).ln()
        })
    } else {
        None
    };
    let probability_drift = request
        .previous_probability
        .map(|previous| request.probability - previous);
    Ok(ForecastScoreMetrics {
        brier: err * err,
        log_loss,
        log_loss_defined: log_loss.is_some(),
        log_loss_undefined_reason: log_loss
            .is_none()
            .then(|| "probability is exactly 0 or 1".to_string()),
        direction_accuracy: (request.probability >= 0.5) == request.outcome.actual_win,
        calibration_bin: calibration_bin(request.probability, request.calibration_bin_count),
        probability_drift,
        absolute_probability_drift: probability_drift.map(f64::abs),
    })
}

fn calibration_bin(probability: f64, count: u32) -> CalibrationBin {
    let mut index = (probability * f64::from(count)).floor() as u32;
    if index >= count {
        index = count - 1;
    }
    CalibrationBin {
        index,
        count,
        lower_inclusive: f64::from(index) / f64::from(count),
        upper_exclusive: f64::from(index + 1) / f64::from(count),
    }
}

fn score_payload(
    request: &ForecastScoreRequest,
    metrics: &ForecastScoreMetrics,
    forecast_hash: &str,
    outcome_hash: &str,
    score_hash: &str,
) -> Result<Vec<u8>> {
    serde_json::to_vec(&json!({
        "schema_version": FORECAST_SCORE_SCHEMA_VERSION,
        "score_id": request.score_id,
        "forecast_ref": safe_ref(&request.forecast_id),
        "forecast_version": request.forecast_version,
        "market_ref": safe_ref(&request.market_id),
        "outcome_ref": safe_ref(&request.outcome_id),
        "source": request.source.as_str(),
        "forecast_hash": forecast_hash,
        "outcome_hash": outcome_hash,
        "score_hash": score_hash,
        "brier": metrics.brier,
        "direction_accuracy": metrics.direction_accuracy
    }))
    .map_err(|err| {
        PolyError::score(
            "CALYX_POLY_SCORE_PAYLOAD_ENCODE_FAILED",
            format!("encode score ledger payload: {err}"),
        )
    })
}

fn forecast_artifact(request: &ForecastScoreRequest) -> Value {
    json!({
        "forecast_id": request.forecast_id,
        "forecast_version": request.forecast_version,
        "market_id": request.market_id,
        "outcome_id": request.outcome_id,
        "source": request.source.as_str(),
        "provider": request.provider,
        "probability": request.probability,
        "confidence": request.confidence,
        "forecast_ts": request.forecast_ts,
        "scored_ts": request.scored_ts,
        "horizon_secs": request.horizon_secs,
        "sufficiency_state": request.sufficiency_state,
        "previous_probability": request.previous_probability,
        "forecast_artifact_hash": request.forecast_artifact_hash
    })
}

fn score_subject(request: &ForecastScoreRequest) -> SubjectId {
    let digest = blake3::hash(
        format!(
            "poly-score:{}:{}:{}",
            request.forecast_id, request.forecast_version, request.outcome_id
        )
        .as_bytes(),
    );
    SubjectId::Query(digest.as_bytes().to_vec())
}

fn validate_score_id(field: &str, value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 32
        || !value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
    {
        return Err(PolyError::score(
            "CALYX_POLY_SCORE_INVALID_ID",
            format!("{field} must be 1..=32 ASCII letters, numbers, hyphen, or underscore"),
        ));
    }
    Ok(())
}

fn validate_public_id(field: &str, value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 256
        || !value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
    {
        return Err(PolyError::score(
            "CALYX_POLY_SCORE_INVALID_ID",
            format!("{field} must be 1..=256 ASCII letters, numbers, hyphen, or underscore"),
        ));
    }
    Ok(())
}

fn validate_label(field: &str, value: Option<&str>) -> Result<()> {
    let Some(value) = value else {
        return Ok(());
    };
    if value.is_empty()
        || value.len() > 32
        || !value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '.')
    {
        return Err(PolyError::score(
            "CALYX_POLY_SCORE_INVALID_LABEL",
            format!("{field} must be 1..=32 ASCII label characters"),
        ));
    }
    Ok(())
}

fn validate_unit(value: f64, field: &str) -> Result<()> {
    if !value.is_finite() || !(0.0..=1.0).contains(&value) {
        return Err(PolyError::score(
            "CALYX_POLY_SCORE_MALFORMED_FORECAST",
            format!("{field} must be finite and in [0, 1]"),
        ));
    }
    Ok(())
}

/// Validates a forecast **confidence** against the handbook ceiling. Grounded confidence is
/// `n/(n+1)`, which never reaches 1, and the forecast confidence ceiling is
/// `min(raw, self-consistency, DPI)` — all strictly below 1. A stored `confidence == 1.0` records
/// an information-theoretically impossible certainty (log-loss `+inf` on a wrong `p=1` forecast),
/// so confidence must be finite in `[0, 1)`. Threading the full grounded `n/(n+1)` cap is tracked
/// in #87; this persistence boundary enforces the strict upper bound `< 1`.
fn validate_confidence(value: f64) -> Result<()> {
    if !value.is_finite() || !(0.0..1.0).contains(&value) {
        return Err(PolyError::score(
            "CALYX_POLY_SCORE_CONFIDENCE_CEILING",
            format!(
                "confidence must be finite and in [0, 1); grounded confidence is n/(n+1) and \
                 never reaches 1 (got {value})"
            ),
        ));
    }
    Ok(())
}

fn validate_hash(field: &str, value: &str) -> Result<()> {
    if value.len() != 64 || !value.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return Err(PolyError::score(
            "CALYX_POLY_SCORE_INVALID_HASH",
            format!("{field} must be a 64-character hex digest"),
        ));
    }
    Ok(())
}

fn write_json_file<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(value).map_err(|err| {
        PolyError::score("CALYX_POLY_SCORE_ARTIFACT_ENCODE_FAILED", err.to_string())
    })?;
    fs::write(path, bytes)
        .map_err(|err| PolyError::score("CALYX_POLY_SCORE_ARTIFACT_WRITE_FAILED", err.to_string()))
}

fn hash_file(path: &Path) -> Result<String> {
    let bytes = fs::read(path).map_err(|err| {
        PolyError::score("CALYX_POLY_SCORE_ARTIFACT_READ_FAILED", err.to_string())
    })?;
    Ok(blake3::hash(&bytes).to_hex().to_string())
}

fn rel_path(path: &str) -> String {
    PathBuf::from(path).display().to_string()
}
