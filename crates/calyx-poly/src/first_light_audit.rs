//! First-light end-to-end artifact-chain audit (issue #237).
//!
//! This module does not produce forecasts. It verifies that a claimed first-light run is backed by
//! the required local sources of truth: a pre-resolution capture state, a persisted CalyxNative
//! forecast artifact, score artifacts, and retune reports. The audit fails closed when a baseline
//! capture is presented as CalyxNative, when timing leaks the outcome, or when retune artifacts do
//! not show a real weight/slope move.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::blend_relearning::{BlendRelearningReport, read_blend_relearning_report};
use crate::calibration_refit::{CalibrationRefitReport, read_calibration_refit_report};
use crate::calyx_native::{CalyxNativeForecast, read_calyx_native_forecast};
use crate::crypto_capture_harness::{CryptoCaptureHarnessState, read_crypto_capture_state};
use crate::diagnostics_store::{read_json, write_json};
use crate::error::{PolyError, Result};
use crate::score::{ForecastScoreManifest, ForecastScoreMetrics, ForecastSource, ResolvedOutcome};

pub const FIRST_LIGHT_AUDIT_SCHEMA_VERSION: &str = "poly.first_light_audit.v1";
pub const FIRST_LIGHT_AUDIT_ARTIFACT_KIND: &str = "poly_first_light_audit";
pub const FIRST_LIGHT_AUDIT_REPORT_FILE: &str = "first_light_audit_report.json";

pub const ERR_FIRST_LIGHT_INVALID: &str = "CALYX_POLY_FIRST_LIGHT_INVALID";
pub const ERR_FIRST_LIGHT_BASELINE_ONLY: &str = "CALYX_POLY_FIRST_LIGHT_BASELINE_ONLY";
pub const ERR_FIRST_LIGHT_LOOKAHEAD: &str = "CALYX_POLY_FIRST_LIGHT_LOOKAHEAD";
pub const ERR_FIRST_LIGHT_MISSING_CHAIN: &str = "CALYX_POLY_FIRST_LIGHT_MISSING_CHAIN";
pub const ERR_FIRST_LIGHT_RETUNE_NO_MOVE: &str = "CALYX_POLY_FIRST_LIGHT_RETUNE_NO_MOVE";
pub const ERR_FIRST_LIGHT_READBACK: &str = "CALYX_POLY_FIRST_LIGHT_READBACK";

pub struct FirstLightAuditRequest<'a> {
    pub report_dir: &'a Path,
    pub capture_state_path: &'a Path,
    pub calyx_native_forecast_path: &'a Path,
    pub score_manifest_path: &'a Path,
    pub blend_relearning_report_path: &'a Path,
    pub calibration_refit_report_path: &'a Path,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FirstLightAuditReport {
    pub schema_version: String,
    pub artifact_kind: String,
    pub source_of_truth: Vec<String>,
    pub market_id: String,
    pub condition_id: String,
    pub token_id: String,
    pub forecast_id: String,
    pub p_model: f64,
    pub confidence: f64,
    pub forecast_ts: u64,
    pub resolved_ts: u64,
    pub actual_win: bool,
    pub brier: f64,
    pub calyx_native_admissible: bool,
    pub calyx_native_refusal_reason: String,
    pub forecast_artifact_blake3: String,
    pub score_id: String,
    pub blend_component_count: usize,
    pub calibration_version: String,
    pub retune_moved: bool,
    pub no_lookahead: bool,
    pub passed: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FirstLightAuditRun {
    pub report_path: PathBuf,
    pub report: FirstLightAuditReport,
}

pub fn run_first_light_audit(request: &FirstLightAuditRequest<'_>) -> Result<FirstLightAuditRun> {
    reject_forbidden_drive(request.report_dir)?;
    let capture = read_crypto_capture_state(request.capture_state_path)?;
    let forecast = read_calyx_native_forecast(request.calyx_native_forecast_path)?;
    let manifest: ForecastScoreManifest = read_json(request.score_manifest_path)?;
    let score_dir = request.score_manifest_path.parent().ok_or_else(|| {
        invalid(
            ERR_FIRST_LIGHT_INVALID,
            "score_manifest_path must have a parent directory",
        )
    })?;
    let score_forecast: Value = read_json(&score_dir.join("forecast.json"))?;
    let outcome: ResolvedOutcome = read_json(&score_dir.join("outcome.json"))?;
    let metrics: ForecastScoreMetrics = read_json(&score_dir.join("score.json"))?;
    let blend = read_blend_relearning_report(request.blend_relearning_report_path)?;
    let calibration = read_calibration_refit_report(request.calibration_refit_report_path)?;
    let forecast_hash = blake3_file(request.calyx_native_forecast_path)?;

    validate_score_source(&manifest, &score_forecast)?;
    validate_score_forecast_hash(&score_forecast, &forecast_hash)?;
    validate_forecast_artifact(&forecast, &manifest, &score_forecast)?;
    let pending = find_calyx_pending(&capture, &manifest.forecast_id)?;
    validate_pending_matches_forecast(pending, &forecast, &manifest)?;
    validate_score_timestamp_matches_pending(pending.forecast_ts, &score_forecast)?;
    validate_score_matches_forecast(&forecast, &manifest, &score_forecast, &outcome)?;
    validate_timing(pending.forecast_ts, outcome.resolved_ts)?;
    let retune_moved = validate_retune_moved(&blend, &calibration)?;

    let report = FirstLightAuditReport {
        schema_version: FIRST_LIGHT_AUDIT_SCHEMA_VERSION.to_string(),
        artifact_kind: FIRST_LIGHT_AUDIT_ARTIFACT_KIND.to_string(),
        source_of_truth: vec![
            request.capture_state_path.display().to_string(),
            request.calyx_native_forecast_path.display().to_string(),
            request.score_manifest_path.display().to_string(),
            score_dir.join("forecast.json").display().to_string(),
            score_dir.join("outcome.json").display().to_string(),
            score_dir.join("score.json").display().to_string(),
            request.blend_relearning_report_path.display().to_string(),
            request.calibration_refit_report_path.display().to_string(),
        ],
        market_id: capture_market_id(&capture, &forecast.condition_id)?,
        condition_id: forecast.condition_id.clone(),
        token_id: forecast.token_id.clone(),
        forecast_id: manifest.forecast_id.clone(),
        p_model: forecast.p_model,
        confidence: forecast.confidence,
        forecast_ts: pending.forecast_ts,
        resolved_ts: outcome.resolved_ts,
        actual_win: outcome.actual_win,
        brier: metrics.brier,
        calyx_native_admissible: forecast.admissible,
        calyx_native_refusal_reason: forecast.refusal_reason.clone(),
        forecast_artifact_blake3: forecast_hash,
        score_id: manifest.score_id.clone(),
        blend_component_count: blend.component_count,
        calibration_version: calibration.version.clone(),
        retune_moved,
        no_lookahead: true,
        passed: true,
    };
    let report_path = write_json(request.report_dir, FIRST_LIGHT_AUDIT_REPORT_FILE, &report)?;
    let readback: FirstLightAuditReport = read_json(&report_path)?;
    if readback != report {
        return Err(invalid(
            ERR_FIRST_LIGHT_READBACK,
            format!(
                "first-light audit report {} did not read back as written",
                report_path.display()
            ),
        ));
    }
    Ok(FirstLightAuditRun {
        report_path,
        report: readback,
    })
}

fn validate_score_source(manifest: &ForecastScoreManifest, forecast: &Value) -> Result<()> {
    if manifest.source != ForecastSource::CalyxNative {
        return Err(invalid(
            ERR_FIRST_LIGHT_BASELINE_ONLY,
            format!(
                "score source is {:?}, expected CalyxNative",
                manifest.source
            ),
        ));
    }
    if forecast.get("source").and_then(Value::as_str) != Some(ForecastSource::CalyxNative.as_str())
    {
        return Err(invalid(
            ERR_FIRST_LIGHT_BASELINE_ONLY,
            "score forecast artifact is not sourced from calyx_native",
        ));
    }
    Ok(())
}

fn validate_forecast_artifact(
    forecast: &CalyxNativeForecast,
    manifest: &ForecastScoreManifest,
    score_forecast: &Value,
) -> Result<()> {
    if forecast.source != ForecastSource::CalyxNative.as_str() {
        return Err(invalid(
            ERR_FIRST_LIGHT_BASELINE_ONLY,
            "forecast artifact is not calyx_native",
        ));
    }
    if forecast.condition_id != value_str(score_forecast, "market_id")?
        || forecast.token_id != value_str(score_forecast, "outcome_id")?
    {
        return Err(invalid(
            ERR_FIRST_LIGHT_MISSING_CHAIN,
            "score forecast market/outcome does not match CalyxNative condition/token",
        ));
    }
    if manifest.outcome_id != forecast.token_id {
        return Err(invalid(
            ERR_FIRST_LIGHT_MISSING_CHAIN,
            "score manifest outcome_id does not match CalyxNative token_id",
        ));
    }
    Ok(())
}

fn validate_score_forecast_hash(score_forecast: &Value, forecast_hash: &str) -> Result<()> {
    if value_str(score_forecast, "forecast_artifact_hash")? != forecast_hash {
        return Err(invalid(
            ERR_FIRST_LIGHT_MISSING_CHAIN,
            "score forecast_artifact_hash does not match CalyxNative forecast file BLAKE3",
        ));
    }
    Ok(())
}

fn validate_pending_matches_forecast(
    pending: &crate::pending_forecast_register::PendingForecastEntry,
    forecast: &CalyxNativeForecast,
    manifest: &ForecastScoreManifest,
) -> Result<()> {
    if pending.source != ForecastSource::CalyxNative {
        return Err(invalid(
            ERR_FIRST_LIGHT_BASELINE_ONLY,
            format!(
                "capture pending entry {} is {:?}, expected CalyxNative",
                pending.forecast_id, pending.source
            ),
        ));
    }
    if pending.condition_id != forecast.condition_id
        || pending.token_id != forecast.token_id
        || pending.provenance_hash != forecast.provenance_hash
        || pending.forecast_version != manifest.forecast_version
    {
        return Err(invalid(
            ERR_FIRST_LIGHT_MISSING_CHAIN,
            "capture pending entry does not match CalyxNative forecast provenance",
        ));
    }
    if !close(pending.p_model, forecast.p_model) || !close(pending.confidence, forecast.confidence)
    {
        return Err(invalid(
            ERR_FIRST_LIGHT_MISSING_CHAIN,
            "capture pending probability/confidence does not match CalyxNative forecast",
        ));
    }
    Ok(())
}

fn validate_score_matches_forecast(
    forecast: &CalyxNativeForecast,
    manifest: &ForecastScoreManifest,
    score_forecast: &Value,
    outcome: &ResolvedOutcome,
) -> Result<()> {
    if !outcome.resolved {
        return Err(invalid(
            ERR_FIRST_LIGHT_MISSING_CHAIN,
            "score outcome is not resolved",
        ));
    }
    if !close(value_f64(score_forecast, "probability")?, forecast.p_model)
        || !close(
            value_f64(score_forecast, "confidence")?,
            forecast.confidence,
        )
    {
        return Err(invalid(
            ERR_FIRST_LIGHT_MISSING_CHAIN,
            "score forecast probability/confidence does not match CalyxNative forecast",
        ));
    }
    if manifest.forecast_version != value_u64(score_forecast, "forecast_version")? as u32 {
        return Err(invalid(
            ERR_FIRST_LIGHT_MISSING_CHAIN,
            "score manifest and forecast artifact versions differ",
        ));
    }
    Ok(())
}

fn validate_score_timestamp_matches_pending(
    forecast_ts: u64,
    score_forecast: &Value,
) -> Result<()> {
    let score_ts = value_u64(score_forecast, "forecast_ts")?;
    if score_ts != forecast_ts {
        return Err(invalid(
            ERR_FIRST_LIGHT_MISSING_CHAIN,
            format!(
                "score forecast_ts {score_ts} does not match captured pending ts {forecast_ts}"
            ),
        ));
    }
    Ok(())
}

fn validate_timing(forecast_ts: u64, resolved_ts: u64) -> Result<()> {
    if forecast_ts >= resolved_ts {
        return Err(invalid(
            ERR_FIRST_LIGHT_LOOKAHEAD,
            format!("forecast_ts {forecast_ts} must be before resolved_ts {resolved_ts}"),
        ));
    }
    Ok(())
}

fn validate_retune_moved(
    blend: &BlendRelearningReport,
    calibration: &CalibrationRefitReport,
) -> Result<bool> {
    let count = blend.component_count;
    if count < 2 || blend.total_reliability_weight <= 0.0 {
        return Err(invalid(
            ERR_FIRST_LIGHT_RETUNE_NO_MOVE,
            "blend relearning needs at least two positively weighted components",
        ));
    }
    let equal = 1.0 / count as f64;
    let weights_moved = blend
        .rows
        .iter()
        .any(|row| (row.normalized_weight - equal).abs() > 1e-9);
    let slope_moved = calibration.brier_improvement > 0.0
        && (calibration.slope.a.abs() > 1e-9 || (calibration.slope.b - 1.0).abs() > 1e-9);
    if !weights_moved || !slope_moved {
        return Err(invalid(
            ERR_FIRST_LIGHT_RETUNE_NO_MOVE,
            "retune artifacts did not show both blend-weight and calibration-slope movement",
        ));
    }
    Ok(true)
}

fn find_calyx_pending<'a>(
    state: &'a CryptoCaptureHarnessState,
    forecast_id: &str,
) -> Result<&'a crate::pending_forecast_register::PendingForecastEntry> {
    state
        .captures
        .iter()
        .flat_map(|capture| capture.snapshots.iter())
        .map(|snapshot| &snapshot.pending_entry)
        .find(|entry| entry.forecast_id == forecast_id)
        .ok_or_else(|| {
            invalid(
                ERR_FIRST_LIGHT_MISSING_CHAIN,
                format!("capture state has no pending entry for forecast {forecast_id}"),
            )
        })
}

fn capture_market_id(state: &CryptoCaptureHarnessState, condition_id: &str) -> Result<String> {
    state
        .captures
        .iter()
        .find(|capture| capture.condition_id == condition_id)
        .map(|capture| capture.market_id.clone())
        .ok_or_else(|| {
            invalid(
                ERR_FIRST_LIGHT_MISSING_CHAIN,
                format!("capture state has no market for condition {condition_id}"),
            )
        })
}

fn value_str<'a>(value: &'a Value, field: &str) -> Result<&'a str> {
    value
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| invalid(ERR_FIRST_LIGHT_MISSING_CHAIN, format!("missing {field}")))
}

fn value_f64(value: &Value, field: &str) -> Result<f64> {
    value
        .get(field)
        .and_then(Value::as_f64)
        .ok_or_else(|| invalid(ERR_FIRST_LIGHT_MISSING_CHAIN, format!("missing {field}")))
}

fn value_u64(value: &Value, field: &str) -> Result<u64> {
    value
        .get(field)
        .and_then(Value::as_u64)
        .ok_or_else(|| invalid(ERR_FIRST_LIGHT_MISSING_CHAIN, format!("missing {field}")))
}

fn blake3_file(path: &Path) -> Result<String> {
    let bytes = fs::read(path).map_err(|err| {
        invalid(
            ERR_FIRST_LIGHT_READBACK,
            format!("read {} for BLAKE3: {err}", path.display()),
        )
    })?;
    Ok(blake3::hash(&bytes).to_hex().to_string())
}

fn reject_forbidden_drive(path: &Path) -> Result<()> {
    let text = path.display().to_string().replace('/', "\\");
    if text.to_ascii_lowercase().starts_with("d:\\") {
        return Err(invalid(
            ERR_FIRST_LIGHT_INVALID,
            format!("D: drive is forbidden for first-light evidence: {text}"),
        ));
    }
    Ok(())
}

fn close(left: f64, right: f64) -> bool {
    (left - right).abs() <= 1e-12
}

fn invalid(code: &'static str, message: impl Into<String>) -> PolyError {
    PolyError::diagnostics(code, message.into())
}
