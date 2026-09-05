//! Regime detection and regime-conditioned forecast parameters (issue #109).

use std::path::{Path, PathBuf};

use calyx_assay::{
    ChangePointReport, CusumConfig, CusumReport, MmdConfig, TrustTag, mmd_change_point,
    recurrence_rate_cusum_with_config,
};
use serde::{Deserialize, Serialize};

use crate::diagnostics_store::{read_json, write_json};
use crate::error::{PolyError, Result};

pub const REGIME_DETECTION_SCHEMA_VERSION: &str = "poly.regime_detection.v1";
pub const REGIME_DETECTION_ARTIFACT_KIND: &str = "poly_regime_detection";
pub const REGIME_DETECTION_REPORT_FILE: &str = "regime_detection_report.json";

pub const ERR_REGIME_DETECTION_INVALID_REQUEST: &str =
    "CALYX_POLY_REGIME_DETECTION_INVALID_REQUEST";
pub const ERR_REGIME_DETECTION_PROVISIONAL_SOURCE: &str =
    "CALYX_POLY_REGIME_DETECTION_PROVISIONAL_SOURCE";
pub const ERR_REGIME_DETECTION_READBACK_MISMATCH: &str =
    "CALYX_POLY_REGIME_DETECTION_READBACK_MISMATCH";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RegimeObservation {
    pub observed_at: f64,
    pub features: Vec<f64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RegimeDetectionRequest {
    pub domain: String,
    pub horizon_bucket: String,
    pub source_artifact: String,
    pub source_trust: TrustTag,
    pub observations: Vec<RegimeObservation>,
    pub mmd_min_window: usize,
    pub mmd_config: MmdConfig,
    pub cusum_config: CusumConfig,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MarketRegime {
    Stable,
    DistributionShift,
    EventRateShift,
    CompoundShift,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RegimeDetectorStatus {
    Activated,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ForecastQualityParameters {
    pub min_p_win: f64,
    pub target_far: f64,
    pub alpha: f64,
    pub min_grounding_anchors: u32,
    pub max_daily_error_score: f64,
    pub knn_k: usize,
    pub encoder_sigma_scale: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RegimeParameterSet {
    pub regime: MarketRegime,
    pub forecast_strategy: String,
    pub parameters: ForecastQualityParameters,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RegimeDetectionReport {
    pub schema_version: String,
    pub artifact_kind: String,
    pub domain: String,
    pub horizon_bucket: String,
    pub source_artifact: String,
    pub source_trust: TrustTag,
    pub observation_count: usize,
    pub feature_dimension: usize,
    pub status: RegimeDetectorStatus,
    pub active_regime: MarketRegime,
    pub active_strategy: String,
    pub active_parameters: ForecastQualityParameters,
    pub parameter_sets: Vec<RegimeParameterSet>,
    pub mmd_change_point: ChangePointReport,
    pub cusum_change_point: CusumReport,
    pub active_change_index: Option<usize>,
    pub detection_hash: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RegimeDetectionRun {
    pub report_path: PathBuf,
    pub report: RegimeDetectionReport,
}

pub fn run_regime_detection_report(
    request: &RegimeDetectionRequest,
    output_root: &Path,
) -> Result<RegimeDetectionRun> {
    let report = compute_regime_detection_report(request)?;
    let report_path = write_regime_detection_report(output_root, &report)?;
    let readback = read_regime_detection_report(&report_path)?;
    let json_normalized_report = json_normalized_report(&report)?;
    if readback != json_normalized_report {
        return Err(PolyError::diagnostics(
            ERR_REGIME_DETECTION_READBACK_MISMATCH,
            format!(
                "regime detection report {} did not read back as written",
                report_path.display()
            ),
        ));
    }
    Ok(RegimeDetectionRun {
        report_path,
        report: readback,
    })
}

fn json_normalized_report(report: &RegimeDetectionReport) -> Result<RegimeDetectionReport> {
    let bytes = serde_json::to_vec(report).map_err(|err| {
        PolyError::diagnostics(
            ERR_REGIME_DETECTION_READBACK_MISMATCH,
            format!("normalize regime detection report through JSON serializer: {err}"),
        )
    })?;
    serde_json::from_slice(&bytes).map_err(|err| {
        PolyError::diagnostics(
            ERR_REGIME_DETECTION_READBACK_MISMATCH,
            format!("normalize regime detection report through JSON deserializer: {err}"),
        )
    })
}

pub fn compute_regime_detection_report(
    request: &RegimeDetectionRequest,
) -> Result<RegimeDetectionReport> {
    validate_request(request)?;
    if request.source_trust != TrustTag::Trusted {
        return Err(PolyError::diagnostics(
            ERR_REGIME_DETECTION_PROVISIONAL_SOURCE,
            "regime-conditioned parameters require trusted source readback",
        ));
    }

    let samples = request
        .observations
        .iter()
        .map(|row| row.features.clone())
        .collect::<Vec<_>>();
    let timestamps = request
        .observations
        .iter()
        .map(|row| row.observed_at)
        .collect::<Vec<_>>();
    let mmd = mmd_change_point(&samples, request.mmd_min_window, &request.mmd_config)?;
    let cusum = recurrence_rate_cusum_with_config(&timestamps, &request.cusum_config)?;
    let active_regime = active_regime(&mmd, &cusum);
    let parameter_sets = default_parameter_sets();
    let active = parameter_sets
        .iter()
        .find(|set| set.regime == active_regime)
        .expect("all regimes have parameter sets");
    let active_change_index = active_change_index(&mmd, &cusum);
    let detection_hash = detection_hash(request, active_regime, active_change_index);

    Ok(RegimeDetectionReport {
        schema_version: REGIME_DETECTION_SCHEMA_VERSION.to_string(),
        artifact_kind: REGIME_DETECTION_ARTIFACT_KIND.to_string(),
        domain: request.domain.clone(),
        horizon_bucket: request.horizon_bucket.clone(),
        source_artifact: request.source_artifact.clone(),
        source_trust: request.source_trust,
        observation_count: request.observations.len(),
        feature_dimension: request.observations[0].features.len(),
        status: RegimeDetectorStatus::Activated,
        active_regime,
        active_strategy: active.forecast_strategy.clone(),
        active_parameters: active.parameters.clone(),
        parameter_sets,
        mmd_change_point: mmd,
        cusum_change_point: cusum,
        active_change_index,
        detection_hash,
    })
}

pub fn write_regime_detection_report(
    dir: &Path,
    report: &RegimeDetectionReport,
) -> Result<PathBuf> {
    write_json(dir, REGIME_DETECTION_REPORT_FILE, report)
}

pub fn read_regime_detection_report(path: &Path) -> Result<RegimeDetectionReport> {
    read_json(path)
}

fn active_regime(mmd: &ChangePointReport, cusum: &CusumReport) -> MarketRegime {
    match (mmd.report.significant, cusum.change_point.is_some()) {
        (true, true) => MarketRegime::CompoundShift,
        (true, false) => MarketRegime::DistributionShift,
        (false, true) => MarketRegime::EventRateShift,
        (false, false) => MarketRegime::Stable,
    }
}

fn active_change_index(mmd: &ChangePointReport, cusum: &CusumReport) -> Option<usize> {
    let mmd_idx = mmd.report.significant.then_some(mmd.split_index);
    let cusum_idx = cusum.change_point.map(|cp| cp.occurrence_index);
    match (mmd_idx, cusum_idx) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (None, None) => None,
    }
}

fn default_parameter_sets() -> Vec<RegimeParameterSet> {
    vec![
        set(
            MarketRegime::Stable,
            "baseline_forecast_quality",
            ForecastQualityParameters {
                min_p_win: 0.90,
                target_far: 0.10,
                alpha: 0.05,
                min_grounding_anchors: 50,
                max_daily_error_score: 10.0,
                knn_k: 11,
                encoder_sigma_scale: 1.0,
            },
        ),
        set(
            MarketRegime::DistributionShift,
            "distribution_shift_conservative_forecast",
            ForecastQualityParameters {
                min_p_win: 0.94,
                target_far: 0.05,
                alpha: 0.025,
                min_grounding_anchors: 75,
                max_daily_error_score: 5.0,
                knn_k: 7,
                encoder_sigma_scale: 0.85,
            },
        ),
        set(
            MarketRegime::EventRateShift,
            "event_rate_shift_fast_recalibration",
            ForecastQualityParameters {
                min_p_win: 0.93,
                target_far: 0.06,
                alpha: 0.03,
                min_grounding_anchors: 75,
                max_daily_error_score: 6.0,
                knn_k: 7,
                encoder_sigma_scale: 0.90,
            },
        ),
        set(
            MarketRegime::CompoundShift,
            "compound_shift_strict_forecast_refusal_bias",
            ForecastQualityParameters {
                min_p_win: 0.96,
                target_far: 0.03,
                alpha: 0.015,
                min_grounding_anchors: 100,
                max_daily_error_score: 3.0,
                knn_k: 5,
                encoder_sigma_scale: 0.75,
            },
        ),
    ]
}

fn set(
    regime: MarketRegime,
    strategy: &str,
    parameters: ForecastQualityParameters,
) -> RegimeParameterSet {
    RegimeParameterSet {
        regime,
        forecast_strategy: strategy.to_string(),
        parameters,
    }
}

fn validate_request(request: &RegimeDetectionRequest) -> Result<()> {
    validate_label("domain", &request.domain)?;
    validate_label("horizon_bucket", &request.horizon_bucket)?;
    validate_label("source_artifact", &request.source_artifact)?;
    if request.mmd_min_window < 4 {
        return invalid("mmd_min_window must be at least 4");
    }
    if request.observations.len() < request.mmd_min_window * 2 {
        return invalid(format!(
            "regime detection needs at least {} observations for MMD",
            request.mmd_min_window * 2
        ));
    }
    if request.observations.len() < 5 {
        return invalid("regime detection needs at least 5 observations for CUSUM");
    }
    let dimension = request.observations[0].features.len();
    if dimension == 0 {
        return invalid("regime feature vectors must not be empty");
    }
    let mut previous = f64::NEG_INFINITY;
    for (idx, row) in request.observations.iter().enumerate() {
        if !row.observed_at.is_finite() || row.observed_at <= previous {
            return invalid(format!(
                "observation {idx} timestamp must be finite and increasing"
            ));
        }
        if row.features.len() != dimension {
            return invalid(format!("observation {idx} feature dimension mismatch"));
        }
        if row.features.iter().any(|value| !value.is_finite()) {
            return invalid(format!("observation {idx} contains a non-finite feature"));
        }
        previous = row.observed_at;
    }
    Ok(())
}

fn detection_hash(
    request: &RegimeDetectionRequest,
    active_regime: MarketRegime,
    active_change_index: Option<usize>,
) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(request.domain.as_bytes());
    hasher.update(request.horizon_bucket.as_bytes());
    hasher.update(format!("{:?}", active_regime).as_bytes());
    hasher.update(&(active_change_index.unwrap_or(usize::MAX) as u64).to_le_bytes());
    for row in &request.observations {
        hasher.update(&row.observed_at.to_le_bytes());
        for value in &row.features {
            hasher.update(&value.to_le_bytes());
        }
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
        ERR_REGIME_DETECTION_INVALID_REQUEST,
        message.into(),
    ))
}
