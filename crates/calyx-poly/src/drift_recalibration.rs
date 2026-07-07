//! Drift-triggered forecast recalibration records (issue #107).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::calibration_refit::CalibrationRefitReport;
use crate::diagnostics_store::{read_json, write_json};
use crate::error::{PolyError, Result};

pub const DRIFT_RECALIBRATION_SCHEMA_VERSION: &str = "poly.drift_recalibration.v1";
pub const DRIFT_RECALIBRATION_ARTIFACT_KIND: &str = "poly_drift_recalibration";
pub const DRIFT_RECALIBRATION_REPORT_FILE: &str = "drift_recalibration_report.json";

pub const ERR_DRIFT_RECALIBRATION_INVALID_REQUEST: &str =
    "CALYX_POLY_DRIFT_RECALIBRATION_INVALID_REQUEST";
pub const ERR_DRIFT_RECALIBRATION_INSUFFICIENT_ANCHORS: &str =
    "CALYX_POLY_DRIFT_RECALIBRATION_INSUFFICIENT_ANCHORS";
pub const ERR_DRIFT_RECALIBRATION_READBACK_MISMATCH: &str =
    "CALYX_POLY_DRIFT_RECALIBRATION_READBACK_MISMATCH";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DriftMetricWindow {
    pub window_id: String,
    pub window_start_millis: u64,
    pub window_end_millis: u64,
    pub input_mmd_p_value: f64,
    pub association_recall_ratio: f64,
    pub calibration_abs_error: f64,
    pub mean_brier: f64,
    pub source_drift_score: f64,
    pub anchor_count: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct DriftRecalibrationThresholds {
    pub max_mmd_p_value: f64,
    pub min_association_recall_ratio: f64,
    pub max_calibration_abs_error: f64,
    pub max_mean_brier: f64,
    pub max_source_drift_score: f64,
    pub min_anchor_count: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct AdmissionConfigSnapshot {
    pub min_p_win: f64,
    pub target_far: f64,
    pub alpha: f64,
    pub min_grounding_anchors: u32,
    pub max_daily_error_score: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DriftRecalibrationRequest {
    pub domain: String,
    pub horizon_bucket: String,
    pub metrics_artifact: String,
    pub calibration_artifact: String,
    pub admission_config_artifact: String,
    pub previous_calibration_version: String,
    pub calibration_refit: CalibrationRefitReport,
    pub admission_before: AdmissionConfigSnapshot,
    pub thresholds: DriftRecalibrationThresholds,
    pub windows: Vec<DriftMetricWindow>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DriftRecalibrationStatus {
    Triggered,
    NotTriggered,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DriftRecalibrationReport {
    pub schema_version: String,
    pub artifact_kind: String,
    pub domain: String,
    pub horizon_bucket: String,
    pub metrics_artifact: String,
    pub calibration_artifact: String,
    pub admission_config_artifact: String,
    pub status: DriftRecalibrationStatus,
    pub trigger_reasons: Vec<String>,
    pub previous_calibration_version: String,
    pub new_calibration_version: String,
    pub calibration_observations_hash: String,
    pub calibration_observation_count: usize,
    pub calibration_brier_improvement: f64,
    pub latest_window: DriftMetricWindow,
    pub admission_before: AdmissionConfigSnapshot,
    pub admission_after: AdmissionConfigSnapshot,
    pub recalibration_record_hash: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DriftRecalibrationRun {
    pub report_path: PathBuf,
    pub report: DriftRecalibrationReport,
}

pub fn run_drift_recalibration_report(
    request: &DriftRecalibrationRequest,
    output_root: &Path,
) -> Result<DriftRecalibrationRun> {
    let report = compute_drift_recalibration_report(request)?;
    let report_path = write_drift_recalibration_report(output_root, &report)?;
    let readback = read_drift_recalibration_report(&report_path)?;
    if serde_json::to_value(&readback).ok() != serde_json::to_value(&report).ok() {
        return Err(PolyError::diagnostics(
            ERR_DRIFT_RECALIBRATION_READBACK_MISMATCH,
            format!(
                "drift recalibration report {} did not read back as written",
                report_path.display()
            ),
        ));
    }
    Ok(DriftRecalibrationRun {
        report_path,
        report: readback,
    })
}

pub fn compute_drift_recalibration_report(
    request: &DriftRecalibrationRequest,
) -> Result<DriftRecalibrationReport> {
    validate_request(request)?;
    let latest = request.windows.last().expect("validated nonempty").clone();
    if latest.anchor_count < request.thresholds.min_anchor_count {
        return Err(PolyError::diagnostics(
            ERR_DRIFT_RECALIBRATION_INSUFFICIENT_ANCHORS,
            format!(
                "latest drift window has {} anchors; needs {}",
                latest.anchor_count, request.thresholds.min_anchor_count
            ),
        ));
    }
    let reasons = trigger_reasons(&latest, &request.thresholds);
    let status = if reasons.is_empty() {
        DriftRecalibrationStatus::NotTriggered
    } else {
        DriftRecalibrationStatus::Triggered
    };
    let admission_after = if status == DriftRecalibrationStatus::Triggered {
        tighten_admission(request.admission_before, &request.thresholds)
    } else {
        request.admission_before
    };
    let recalibration_record_hash = record_hash(request, &latest, status, &reasons);

    Ok(DriftRecalibrationReport {
        schema_version: DRIFT_RECALIBRATION_SCHEMA_VERSION.to_string(),
        artifact_kind: DRIFT_RECALIBRATION_ARTIFACT_KIND.to_string(),
        domain: request.domain.clone(),
        horizon_bucket: request.horizon_bucket.clone(),
        metrics_artifact: request.metrics_artifact.clone(),
        calibration_artifact: request.calibration_artifact.clone(),
        admission_config_artifact: request.admission_config_artifact.clone(),
        status,
        trigger_reasons: reasons,
        previous_calibration_version: request.previous_calibration_version.clone(),
        new_calibration_version: request.calibration_refit.version.clone(),
        calibration_observations_hash: request.calibration_refit.observations_hash.clone(),
        calibration_observation_count: request.calibration_refit.observation_count,
        calibration_brier_improvement: request.calibration_refit.brier_improvement,
        latest_window: latest,
        admission_before: request.admission_before,
        admission_after,
        recalibration_record_hash,
    })
}

pub fn write_drift_recalibration_report(
    dir: &Path,
    report: &DriftRecalibrationReport,
) -> Result<PathBuf> {
    write_json(dir, DRIFT_RECALIBRATION_REPORT_FILE, report)
}

pub fn read_drift_recalibration_report(path: &Path) -> Result<DriftRecalibrationReport> {
    read_json(path)
}

fn trigger_reasons(
    latest: &DriftMetricWindow,
    thresholds: &DriftRecalibrationThresholds,
) -> Vec<String> {
    let mut reasons = Vec::new();
    if latest.input_mmd_p_value <= thresholds.max_mmd_p_value {
        reasons.push("input_distribution_mmd".to_string());
    }
    if latest.association_recall_ratio < thresholds.min_association_recall_ratio {
        reasons.push("association_recall".to_string());
    }
    if latest.calibration_abs_error > thresholds.max_calibration_abs_error {
        reasons.push("calibration_residual".to_string());
    }
    if latest.mean_brier > thresholds.max_mean_brier {
        reasons.push("outcome_scoring_brier".to_string());
    }
    if latest.source_drift_score > thresholds.max_source_drift_score {
        reasons.push("source_drift_score".to_string());
    }
    reasons
}

fn tighten_admission(
    before: AdmissionConfigSnapshot,
    thresholds: &DriftRecalibrationThresholds,
) -> AdmissionConfigSnapshot {
    AdmissionConfigSnapshot {
        min_p_win: (before.min_p_win + 0.03).min(0.99),
        target_far: before.target_far * 0.5,
        alpha: before.alpha * 0.5,
        min_grounding_anchors: before
            .min_grounding_anchors
            .max(thresholds.min_anchor_count as u32),
        max_daily_error_score: before.max_daily_error_score * 0.5,
    }
}

fn validate_request(request: &DriftRecalibrationRequest) -> Result<()> {
    for (field, value) in [
        ("domain", &request.domain),
        ("horizon_bucket", &request.horizon_bucket),
        ("metrics_artifact", &request.metrics_artifact),
        ("calibration_artifact", &request.calibration_artifact),
        (
            "admission_config_artifact",
            &request.admission_config_artifact,
        ),
        (
            "previous_calibration_version",
            &request.previous_calibration_version,
        ),
    ] {
        validate_label(field, value)?;
    }
    if request.windows.is_empty() {
        return invalid("drift recalibration requires at least one metric window");
    }
    validate_thresholds(request.thresholds)?;
    validate_admission(request.admission_before)?;
    if request.calibration_refit.previous_version.as_deref()
        != Some(request.previous_calibration_version.as_str())
    {
        return invalid("calibration refit previous_version must match the request");
    }
    for window in &request.windows {
        validate_window(window)?;
    }
    Ok(())
}

fn validate_window(window: &DriftMetricWindow) -> Result<()> {
    validate_label("window_id", &window.window_id)?;
    if window.window_start_millis >= window.window_end_millis {
        return invalid("metric window start must be before end");
    }
    for (name, value) in [
        ("input_mmd_p_value", window.input_mmd_p_value),
        ("association_recall_ratio", window.association_recall_ratio),
        ("calibration_abs_error", window.calibration_abs_error),
        ("mean_brier", window.mean_brier),
        ("source_drift_score", window.source_drift_score),
    ] {
        if !value.is_finite() {
            return invalid(format!("{name} must be finite"));
        }
    }
    if !(0.0..=1.0).contains(&window.input_mmd_p_value)
        || !(0.0..=1.0).contains(&window.association_recall_ratio)
    {
        return invalid("MMD p-value and association recall must be in [0,1]");
    }
    Ok(())
}

fn validate_thresholds(thresholds: DriftRecalibrationThresholds) -> Result<()> {
    if thresholds.min_anchor_count == 0 {
        return invalid("min_anchor_count must be positive");
    }
    for (name, value) in [
        ("max_mmd_p_value", thresholds.max_mmd_p_value),
        (
            "min_association_recall_ratio",
            thresholds.min_association_recall_ratio,
        ),
        (
            "max_calibration_abs_error",
            thresholds.max_calibration_abs_error,
        ),
        ("max_mean_brier", thresholds.max_mean_brier),
        ("max_source_drift_score", thresholds.max_source_drift_score),
    ] {
        if !value.is_finite() || value < 0.0 {
            return invalid(format!("{name} must be finite and non-negative"));
        }
    }
    Ok(())
}

fn validate_admission(config: AdmissionConfigSnapshot) -> Result<()> {
    for (name, value) in [
        ("min_p_win", config.min_p_win),
        ("target_far", config.target_far),
        ("alpha", config.alpha),
        ("max_daily_error_score", config.max_daily_error_score),
    ] {
        if !value.is_finite() || value < 0.0 {
            return invalid(format!("{name} must be finite and non-negative"));
        }
    }
    if config.min_grounding_anchors == 0 {
        return invalid("min_grounding_anchors must be positive");
    }
    Ok(())
}

fn record_hash(
    request: &DriftRecalibrationRequest,
    latest: &DriftMetricWindow,
    status: DriftRecalibrationStatus,
    reasons: &[String],
) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(request.domain.as_bytes());
    hasher.update(request.horizon_bucket.as_bytes());
    hasher.update(format!("{:?}", status).as_bytes());
    hasher.update(request.previous_calibration_version.as_bytes());
    hasher.update(request.calibration_refit.version.as_bytes());
    hasher.update(latest.window_id.as_bytes());
    for reason in reasons {
        hasher.update(reason.as_bytes());
    }
    hasher.finalize().to_hex().to_string()
}

fn validate_label(field: &str, value: &str) -> Result<()> {
    if value.trim().is_empty() {
        return invalid(format!("{field} must not be empty"));
    }
    Ok(())
}

fn invalid<T>(message: impl Into<String>) -> Result<T> {
    Err(PolyError::diagnostics(
        ERR_DRIFT_RECALIBRATION_INVALID_REQUEST,
        message.into(),
    ))
}
