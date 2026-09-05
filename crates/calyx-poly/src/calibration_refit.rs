//! Versioned domain×horizon calibration slope refits (issue #111).
//!
//! This module wraps the existing deterministic calibration fitter, assigns a durable version to the
//! refit, persists the report, and reads it back. It fails closed on stale/future observations,
//! insufficient samples, single-class histories, non-finite probabilities, and non-improving fits.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::diagnostics_store::{read_json, write_json};
use crate::forecast_calibration::{CalibrationSlope, fit_calibration_slope};
use crate::{PolyError, Result};

pub const CALIBRATION_REFIT_SCHEMA_VERSION: &str = "poly.calibration_refit.v1";
pub const CALIBRATION_REFIT_ARTIFACT_KIND: &str = "poly_calibration_refit_report";
pub const CALIBRATION_REFIT_REPORT_FILE: &str = "calibration_refit_report.json";

pub const ERR_CALIBRATION_REFIT_INVALID: &str = "CALYX_POLY_CALIBRATION_REFIT_INVALID";
pub const ERR_CALIBRATION_REFIT_FUTURE_OBSERVATION: &str =
    "CALYX_POLY_CALIBRATION_REFIT_FUTURE_OBSERVATION";
pub const ERR_CALIBRATION_REFIT_NO_IMPROVEMENT: &str =
    "CALYX_POLY_CALIBRATION_REFIT_NO_IMPROVEMENT";
pub const ERR_CALIBRATION_REFIT_READBACK_MISMATCH: &str =
    "CALYX_POLY_CALIBRATION_REFIT_READBACK_MISMATCH";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CalibrationRefitObservation {
    pub p_raw: f64,
    pub outcome_yes: bool,
    pub resolved_at_millis: u64,
}

pub struct CalibrationRefitRequest<'a> {
    pub out_dir: &'a Path,
    pub domain: &'a str,
    pub horizon_bucket: &'a str,
    pub previous_version: Option<&'a str>,
    pub as_of_millis: u64,
    pub observations: Vec<CalibrationRefitObservation>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CalibrationRefitReport {
    pub schema_version: String,
    pub artifact_kind: String,
    pub version: String,
    pub previous_version: Option<String>,
    pub as_of_millis: u64,
    pub observations_hash: String,
    pub observation_count: usize,
    pub positives: usize,
    pub slope: CalibrationSlope,
    pub brier_improvement: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CalibrationRefitRun {
    pub report_path: PathBuf,
    pub report: CalibrationRefitReport,
}

pub fn run_calibration_refit(request: &CalibrationRefitRequest<'_>) -> Result<CalibrationRefitRun> {
    let report = compute_calibration_refit_report(request)?;
    let report_path = write_json(request.out_dir, CALIBRATION_REFIT_REPORT_FILE, &report)?;
    let readback = read_calibration_refit_report(&report_path)?;
    if readback != report {
        return Err(PolyError::diagnostics(
            ERR_CALIBRATION_REFIT_READBACK_MISMATCH,
            format!(
                "calibration refit report changed during readback from {}",
                report_path.display()
            ),
        ));
    }
    Ok(CalibrationRefitRun {
        report_path,
        report: readback,
    })
}

pub fn compute_calibration_refit_report(
    request: &CalibrationRefitRequest<'_>,
) -> Result<CalibrationRefitReport> {
    validate_request(request)?;
    let pairs = request
        .observations
        .iter()
        .map(|obs| (obs.p_raw, obs.outcome_yes))
        .collect::<Vec<_>>();
    let mut slope = fit_calibration_slope(request.domain, request.horizon_bucket, &pairs)?;
    let brier_improvement = report_float(slope.brier_raw - slope.brier_calibrated);
    if brier_improvement <= 0.0 {
        return Err(PolyError::diagnostics(
            ERR_CALIBRATION_REFIT_NO_IMPROVEMENT,
            format!(
                "calibration refit did not improve Brier: raw={} calibrated={}",
                slope.brier_raw, slope.brier_calibrated
            ),
        ));
    }
    slope.a = report_float(slope.a);
    slope.b = report_float(slope.b);
    slope.brier_raw = report_float(slope.brier_raw);
    slope.brier_calibrated = report_float(slope.brier_calibrated);
    let observations_hash = observations_hash(request)?;
    let version = format!(
        "{}:{}:{}:{}",
        request.domain,
        request.horizon_bucket,
        request.as_of_millis,
        &observations_hash[..12]
    );
    Ok(CalibrationRefitReport {
        schema_version: CALIBRATION_REFIT_SCHEMA_VERSION.to_string(),
        artifact_kind: CALIBRATION_REFIT_ARTIFACT_KIND.to_string(),
        version,
        previous_version: request.previous_version.map(str::to_string),
        as_of_millis: request.as_of_millis,
        observations_hash,
        observation_count: request.observations.len(),
        positives: request
            .observations
            .iter()
            .filter(|obs| obs.outcome_yes)
            .count(),
        slope,
        brier_improvement,
    })
}

pub fn read_calibration_refit_report(path: &Path) -> Result<CalibrationRefitReport> {
    read_json(path)
}

fn validate_request(request: &CalibrationRefitRequest<'_>) -> Result<()> {
    if request.domain.trim().is_empty() || request.horizon_bucket.trim().is_empty() {
        return Err(PolyError::diagnostics(
            ERR_CALIBRATION_REFIT_INVALID,
            "domain and horizon_bucket are required",
        ));
    }
    for obs in &request.observations {
        if obs.resolved_at_millis > request.as_of_millis {
            return Err(PolyError::diagnostics(
                ERR_CALIBRATION_REFIT_FUTURE_OBSERVATION,
                "calibration refit cannot use an observation after as_of_millis",
            ));
        }
    }
    Ok(())
}

fn report_float(value: f64) -> f64 {
    let rounded = (value * 1_000_000_000_000.0).round() / 1_000_000_000_000.0;
    if rounded == -0.0 { 0.0 } else { rounded }
}

fn observations_hash(request: &CalibrationRefitRequest<'_>) -> Result<String> {
    let bytes = serde_json::to_vec(&request.observations).map_err(|err| {
        PolyError::diagnostics(
            ERR_CALIBRATION_REFIT_INVALID,
            format!("encode calibration observations for hash: {err}"),
        )
    })?;
    Ok(blake3::hash(&bytes).to_hex().to_string())
}
