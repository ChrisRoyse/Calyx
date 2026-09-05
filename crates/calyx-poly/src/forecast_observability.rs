//! Local forecast observability metrics and alert reports (issue #125).
//!
//! This module emits Prometheus-style text artifacts for local forecasting health, reads the emitted
//! metrics back, and persists a JSON report that points to the physical source of truth. Metrics are
//! about forecast quality and system health only; no betting PnL or execution state is represented.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::diagnostics_store::{read_json, write_json};
use crate::error::{PolyError, Result};
use crate::metrics_scrape::{
    ERR_METRICS_ALERT, ERR_METRICS_MISSING_SINK, MetricCheck, MetricSample, read_prometheus_samples,
};

pub const FORECAST_OBSERVABILITY_SCHEMA_VERSION: &str = "poly.forecast_observability.v1";
pub const FORECAST_OBSERVABILITY_ARTIFACT_KIND: &str = "poly_forecast_observability_report";
pub const FORECAST_OBSERVABILITY_REPORT_FILE: &str = "forecast_observability_report.json";
pub const FORECAST_OBSERVABILITY_METRICS_FILE: &str = "forecast_observability.metrics";

pub const METRIC_FORECAST_SCORED_TOTAL: &str = "poly_forecast_scored_total";
pub const METRIC_FORECAST_BRIER_MEAN: &str = "poly_forecast_brier_mean";
pub const METRIC_FORECAST_CALIBRATION_ABS_ERROR_MEAN: &str =
    "poly_forecast_calibration_abs_error_mean";
pub const METRIC_FORECAST_DIRECTION_ACCURACY: &str = "poly_forecast_direction_accuracy";
pub const METRIC_FORECAST_REFUSALS_TOTAL: &str = "poly_forecast_refusals_total";
pub const METRIC_DEEPSEEK_AGENT_FAILURES_TOTAL: &str = "poly_deepseek_agent_failures_total";
pub const METRIC_INGEST_FRESHNESS_SECONDS: &str = "poly_ingest_freshness_seconds";
pub const METRIC_ASSOCIATION_COVERAGE_RATIO: &str = "poly_association_coverage_ratio";

pub const ERR_OBSERVABILITY_INVALID_REQUEST: &str = "CALYX_POLY_OBSERVABILITY_INVALID_REQUEST";
pub const ERR_OBSERVABILITY_STALE_INGEST: &str = "CALYX_POLY_OBSERVABILITY_STALE_INGEST";
pub const ERR_OBSERVABILITY_READBACK_MISMATCH: &str = "CALYX_POLY_OBSERVABILITY_READBACK_MISMATCH";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ForecastObservabilityStatus {
    Healthy,
    Alert,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ForecastQualityMetrics {
    pub scored_forecasts: u64,
    pub mean_brier: f64,
    pub mean_calibration_abs_error: f64,
    pub direction_accuracy: f64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForecastRefusalMetric {
    pub code: String,
    pub count: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ForecastObservabilityThresholds {
    pub max_mean_brier: f64,
    pub max_calibration_abs_error: f64,
    pub max_refusals_total: u64,
    pub max_agent_failures_total: u64,
    pub min_association_coverage_ratio: f64,
}

pub struct ForecastObservabilityRequest<'a> {
    pub source: &'a str,
    pub out_dir: &'a Path,
    pub evaluated_at_millis: u64,
    pub ingest_last_observed_millis: u64,
    pub max_ingest_age_millis: u64,
    pub quality: ForecastQualityMetrics,
    pub refusals: Vec<ForecastRefusalMetric>,
    pub deepseek_agent_failures_total: u64,
    pub association_coverage_ratio: f64,
    pub thresholds: ForecastObservabilityThresholds,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ForecastObservabilityReport {
    pub schema_version: String,
    pub artifact_kind: String,
    pub source: String,
    pub metrics_path: String,
    pub metrics_blake3: String,
    pub evaluated_at_millis: u64,
    pub ingest_last_observed_millis: u64,
    pub ingest_freshness_seconds: f64,
    pub max_ingest_age_millis: u64,
    pub thresholds: ForecastObservabilityThresholds,
    pub quality: ForecastQualityMetrics,
    pub refusals: Vec<ForecastRefusalMetric>,
    pub deepseek_agent_failures_total: u64,
    pub association_coverage_ratio: f64,
    pub samples: Vec<MetricSample>,
    pub checks: Vec<MetricCheck>,
    pub alert_count: usize,
    pub status: ForecastObservabilityStatus,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ForecastObservabilityRun {
    pub metrics_path: PathBuf,
    pub report_path: PathBuf,
    pub report: ForecastObservabilityReport,
}

pub fn run_forecast_observability_report(
    request: &ForecastObservabilityRequest<'_>,
) -> Result<ForecastObservabilityRun> {
    validate_request(request)?;
    std::fs::create_dir_all(request.out_dir).map_err(|err| {
        PolyError::diagnostics(
            ERR_METRICS_MISSING_SINK,
            format!(
                "create observability dir {}: {err}",
                request.out_dir.display()
            ),
        )
    })?;
    let metrics_path = request.out_dir.join(FORECAST_OBSERVABILITY_METRICS_FILE);
    let metrics_text = render_metrics(request)?;
    std::fs::write(&metrics_path, metrics_text.as_bytes()).map_err(|err| {
        PolyError::diagnostics(
            ERR_METRICS_MISSING_SINK,
            format!("write metrics sink {}: {err}", metrics_path.display()),
        )
    })?;

    let report = compute_forecast_observability_report(request, &metrics_path)?;
    let report_path = write_forecast_observability_report(request.out_dir, &report)?;
    let readback = read_forecast_observability_report(&report_path)?;
    if readback != report {
        return Err(PolyError::diagnostics(
            ERR_OBSERVABILITY_READBACK_MISMATCH,
            format!(
                "forecast observability report changed during readback from {}",
                report_path.display()
            ),
        ));
    }
    Ok(ForecastObservabilityRun {
        metrics_path,
        report_path,
        report: readback,
    })
}

pub fn compute_forecast_observability_report(
    request: &ForecastObservabilityRequest<'_>,
    metrics_path: &Path,
) -> Result<ForecastObservabilityReport> {
    validate_request(request)?;
    let samples = read_prometheus_samples(metrics_path)?;
    let metrics_bytes = std::fs::read(metrics_path).map_err(|err| {
        PolyError::diagnostics(
            ERR_METRICS_MISSING_SINK,
            format!("read metrics sink {}: {err}", metrics_path.display()),
        )
    })?;
    let checks = evaluate_observability_checks(&samples, request)?;
    let alert_count = checks.iter().filter(|check| !check.passed).count();
    Ok(ForecastObservabilityReport {
        schema_version: FORECAST_OBSERVABILITY_SCHEMA_VERSION.to_string(),
        artifact_kind: FORECAST_OBSERVABILITY_ARTIFACT_KIND.to_string(),
        source: request.source.to_string(),
        metrics_path: metrics_path.display().to_string(),
        metrics_blake3: blake3::hash(&metrics_bytes).to_hex().to_string(),
        evaluated_at_millis: request.evaluated_at_millis,
        ingest_last_observed_millis: request.ingest_last_observed_millis,
        ingest_freshness_seconds: ingest_age_seconds(request),
        max_ingest_age_millis: request.max_ingest_age_millis,
        thresholds: request.thresholds,
        quality: request.quality,
        refusals: request.refusals.clone(),
        deepseek_agent_failures_total: request.deepseek_agent_failures_total,
        association_coverage_ratio: request.association_coverage_ratio,
        samples,
        checks,
        alert_count,
        status: if alert_count == 0 {
            ForecastObservabilityStatus::Healthy
        } else {
            ForecastObservabilityStatus::Alert
        },
    })
}

pub fn require_forecast_observability_healthy(report: &ForecastObservabilityReport) -> Result<()> {
    if report.status == ForecastObservabilityStatus::Healthy {
        return Ok(());
    }
    let failed = report
        .checks
        .iter()
        .filter(|check| !check.passed)
        .map(|check| check.message.as_str())
        .collect::<Vec<_>>()
        .join("; ");
    Err(PolyError::diagnostics(
        ERR_METRICS_ALERT,
        format!("forecast observability alerts fired: {failed}"),
    ))
}

pub fn read_forecast_observability_metrics(path: &Path) -> Result<Vec<MetricSample>> {
    read_prometheus_samples(path)
}

pub fn write_forecast_observability_report(
    dir: &Path,
    report: &ForecastObservabilityReport,
) -> Result<PathBuf> {
    write_json(dir, FORECAST_OBSERVABILITY_REPORT_FILE, report)
}

pub fn read_forecast_observability_report(path: &Path) -> Result<ForecastObservabilityReport> {
    read_json(path)
}

fn validate_request(request: &ForecastObservabilityRequest<'_>) -> Result<()> {
    if request.source.trim().is_empty() || request.max_ingest_age_millis == 0 {
        return invalid("source and max_ingest_age_millis are required");
    }
    if request.evaluated_at_millis < request.ingest_last_observed_millis {
        return invalid("evaluated_at_millis cannot precede ingest_last_observed_millis");
    }
    let age = request.evaluated_at_millis - request.ingest_last_observed_millis;
    if age > request.max_ingest_age_millis {
        return Err(PolyError::diagnostics(
            ERR_OBSERVABILITY_STALE_INGEST,
            format!(
                "ingest freshness age {age}ms exceeds max {}ms",
                request.max_ingest_age_millis
            ),
        ));
    }
    validate_quality(request.quality)?;
    validate_thresholds(request.thresholds)?;
    if !(0.0..=1.0).contains(&request.association_coverage_ratio)
        || !request.association_coverage_ratio.is_finite()
    {
        return invalid("association_coverage_ratio must be finite in [0, 1]");
    }
    for refusal in &request.refusals {
        validate_label_value(&refusal.code)?;
    }
    Ok(())
}

fn validate_quality(quality: ForecastQualityMetrics) -> Result<()> {
    let valid = quality.scored_forecasts > 0
        && finite_ratio(quality.mean_brier)
        && finite_ratio(quality.mean_calibration_abs_error)
        && finite_ratio(quality.direction_accuracy);
    if valid {
        Ok(())
    } else {
        invalid("forecast quality metrics require scored_forecasts > 0 and finite ratios in [0, 1]")
    }
}

fn validate_thresholds(thresholds: ForecastObservabilityThresholds) -> Result<()> {
    let valid = finite_ratio(thresholds.max_mean_brier)
        && finite_ratio(thresholds.max_calibration_abs_error)
        && finite_ratio(thresholds.min_association_coverage_ratio);
    if valid {
        Ok(())
    } else {
        invalid("forecast observability thresholds must be finite ratios in [0, 1]")
    }
}

fn render_metrics(request: &ForecastObservabilityRequest<'_>) -> Result<String> {
    let source = validate_label_value(request.source)?;
    let mut out = String::new();
    push_metric(
        &mut out,
        METRIC_FORECAST_SCORED_TOTAL,
        &format!("source=\"{source}\""),
        request.quality.scored_forecasts as f64,
    );
    push_metric(
        &mut out,
        METRIC_FORECAST_BRIER_MEAN,
        &format!("source=\"{source}\""),
        request.quality.mean_brier,
    );
    push_metric(
        &mut out,
        METRIC_FORECAST_CALIBRATION_ABS_ERROR_MEAN,
        &format!("source=\"{source}\""),
        request.quality.mean_calibration_abs_error,
    );
    push_metric(
        &mut out,
        METRIC_FORECAST_DIRECTION_ACCURACY,
        &format!("source=\"{source}\""),
        request.quality.direction_accuracy,
    );
    for refusal in &request.refusals {
        let code = validate_label_value(&refusal.code)?;
        push_metric(
            &mut out,
            METRIC_FORECAST_REFUSALS_TOTAL,
            &format!("source=\"{source}\",code=\"{code}\""),
            refusal.count as f64,
        );
    }
    push_metric(
        &mut out,
        METRIC_DEEPSEEK_AGENT_FAILURES_TOTAL,
        &format!("source=\"{source}\""),
        request.deepseek_agent_failures_total as f64,
    );
    push_metric(
        &mut out,
        METRIC_INGEST_FRESHNESS_SECONDS,
        &format!("source=\"{source}\""),
        ingest_age_seconds(request),
    );
    push_metric(
        &mut out,
        METRIC_ASSOCIATION_COVERAGE_RATIO,
        &format!("source=\"{source}\""),
        request.association_coverage_ratio,
    );
    Ok(out)
}

fn evaluate_observability_checks(
    samples: &[MetricSample],
    request: &ForecastObservabilityRequest<'_>,
) -> Result<Vec<MetricCheck>> {
    Ok(vec![
        max_check(
            samples,
            METRIC_FORECAST_BRIER_MEAN,
            request.thresholds.max_mean_brier,
        )?,
        max_check(
            samples,
            METRIC_FORECAST_CALIBRATION_ABS_ERROR_MEAN,
            request.thresholds.max_calibration_abs_error,
        )?,
        max_value_check(
            METRIC_FORECAST_REFUSALS_TOTAL,
            sum_metric(samples, METRIC_FORECAST_REFUSALS_TOTAL)?,
            request.thresholds.max_refusals_total as f64,
        ),
        max_check(
            samples,
            METRIC_DEEPSEEK_AGENT_FAILURES_TOTAL,
            request.thresholds.max_agent_failures_total as f64,
        )?,
        max_check(
            samples,
            METRIC_INGEST_FRESHNESS_SECONDS,
            request.max_ingest_age_millis as f64 / 1_000.0,
        )?,
        min_check(
            samples,
            METRIC_ASSOCIATION_COVERAGE_RATIO,
            request.thresholds.min_association_coverage_ratio,
        )?,
    ])
}

fn max_check(samples: &[MetricSample], metric: &str, threshold: f64) -> Result<MetricCheck> {
    let sample = single_metric(samples, metric)?;
    Ok(check(
        metric,
        sample.value,
        "<=",
        threshold,
        sample.value <= threshold,
    ))
}

fn min_check(samples: &[MetricSample], metric: &str, threshold: f64) -> Result<MetricCheck> {
    let sample = single_metric(samples, metric)?;
    Ok(check(
        metric,
        sample.value,
        ">=",
        threshold,
        sample.value >= threshold,
    ))
}

fn max_value_check(metric: &str, value: f64, threshold: f64) -> MetricCheck {
    check(metric, value, "<=", threshold, value <= threshold)
}

fn single_metric<'a>(samples: &'a [MetricSample], metric: &str) -> Result<&'a MetricSample> {
    let mut selected = samples.iter().filter(|sample| sample.name == metric);
    let Some(sample) = selected.next() else {
        return invalid(format!("required observability metric {metric} is missing"));
    };
    if selected.next().is_some() {
        return invalid(format!(
            "observability metric {metric} must have one sample"
        ));
    }
    Ok(sample)
}

fn sum_metric(samples: &[MetricSample], metric: &str) -> Result<f64> {
    let selected = samples
        .iter()
        .filter(|sample| sample.name == metric)
        .map(|sample| sample.value)
        .collect::<Vec<_>>();
    if selected.is_empty() {
        return invalid(format!("required observability metric {metric} is missing"));
    }
    Ok(selected.iter().sum())
}

fn check(metric: &str, value: f64, comparator: &str, threshold: f64, passed: bool) -> MetricCheck {
    MetricCheck {
        metric: metric.to_string(),
        labels: None,
        value,
        comparator: comparator.to_string(),
        threshold,
        passed,
        code: if passed {
            "CALYX_POLY_METRICS_CHECK_PASSED".to_string()
        } else {
            ERR_METRICS_ALERT.to_string()
        },
        message: format!("{metric}={value} {comparator} {threshold} passed={passed}"),
    }
}

fn push_metric(out: &mut String, name: &str, labels: &str, value: f64) {
    out.push_str(name);
    out.push('{');
    out.push_str(labels);
    out.push_str("} ");
    out.push_str(&format!("{value:.12}"));
    out.push('\n');
}

fn ingest_age_seconds(request: &ForecastObservabilityRequest<'_>) -> f64 {
    (request.evaluated_at_millis - request.ingest_last_observed_millis) as f64 / 1_000.0
}

fn finite_ratio(value: f64) -> bool {
    value.is_finite() && (0.0..=1.0).contains(&value)
}

fn validate_label_value(value: &str) -> Result<String> {
    if value.trim().is_empty()
        || value.contains('"')
        || value.contains('\\')
        || value.contains('\n')
        || value.contains('\r')
    {
        return invalid("metric label values must be non-empty and quote-safe");
    }
    Ok(value.to_string())
}

fn invalid<T>(message: impl Into<String>) -> Result<T> {
    Err(PolyError::diagnostics(
        ERR_OBSERVABILITY_INVALID_REQUEST,
        message.into(),
    ))
}
