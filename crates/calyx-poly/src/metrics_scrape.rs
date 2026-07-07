//! Local `/metrics` scrape evaluation for Calyx-native health gates (issue #126).
//!
//! The source of truth is a captured Prometheus text exposition from `calyxd /metrics`, read from
//! disk, plus the persisted JSON evaluation report. This module is intentionally local-only: it
//! does not run a server, reach trading surfaces, or treat alerts as silent degradation.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::diagnostics_store::{read_json, write_json};
use crate::error::{PolyError, Result};

pub const METRICS_SCRAPE_SCHEMA_VERSION: &str = "poly.metrics_scrape.v1";
pub const METRICS_SCRAPE_ARTIFACT_KIND: &str = "poly_metrics_scrape_report";
pub const METRICS_SCRAPE_REPORT_FILE: &str = "metrics_scrape_report.json";

pub const METRIC_KERNEL_RECALL: &str = "poly_kernel_recall_ratio";
pub const METRIC_GUARD_FAR: &str = "poly_guard_far_ratio";
pub const METRIC_PANEL_N_EFF: &str = "poly_panel_n_eff";
pub const METRIC_CHAIN_VERIFY_PASSED: &str = "poly_chain_verify_passed";

pub const ERR_METRICS_MISSING_SINK: &str = "CALYX_POLY_METRICS_MISSING_SINK";
pub const ERR_METRICS_INVALID_REQUEST: &str = "CALYX_POLY_METRICS_INVALID_REQUEST";
pub const ERR_METRICS_STALE: &str = "CALYX_POLY_METRICS_STALE";
pub const ERR_METRICS_MALFORMED: &str = "CALYX_POLY_METRICS_MALFORMED";
pub const ERR_METRICS_MISSING_REQUIRED: &str = "CALYX_POLY_METRICS_MISSING_REQUIRED";
pub const ERR_METRICS_ALERT: &str = "CALYX_POLY_METRICS_ALERT";
pub const ERR_METRICS_READBACK_MISMATCH: &str = "CALYX_POLY_METRICS_READBACK_MISMATCH";

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct MetricsThresholds {
    pub min_kernel_recall_ratio: f64,
    pub max_guard_far_ratio: f64,
    pub min_n_eff: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MetricsStatus {
    Healthy,
    Alert,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MetricSample {
    pub name: String,
    pub labels: Option<String>,
    pub value: f64,
    pub line_number: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MetricCheck {
    pub metric: String,
    pub labels: Option<String>,
    pub value: f64,
    pub comparator: String,
    pub threshold: f64,
    pub passed: bool,
    pub code: String,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MetricsScrapeReport {
    pub schema_version: String,
    pub artifact_kind: String,
    pub source: String,
    pub metrics_path: String,
    pub metrics_blake3: String,
    pub scraped_at_millis: u64,
    pub evaluated_at_millis: u64,
    pub max_age_millis: u64,
    pub thresholds: MetricsThresholds,
    pub sample_count: usize,
    pub samples: Vec<MetricSample>,
    pub checks: Vec<MetricCheck>,
    pub alert_count: usize,
    pub status: MetricsStatus,
}

pub struct MetricsScrapeRequest<'a> {
    pub source: &'a str,
    pub metrics_path: &'a Path,
    pub out_dir: &'a Path,
    pub scraped_at_millis: u64,
    pub evaluated_at_millis: u64,
    pub max_age_millis: u64,
    pub thresholds: MetricsThresholds,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MetricsScrapeRun {
    pub report_path: PathBuf,
    pub report: MetricsScrapeReport,
}

pub fn run_metrics_scrape_report(request: &MetricsScrapeRequest<'_>) -> Result<MetricsScrapeRun> {
    let report = compute_metrics_scrape_report(request)?;
    let report_path = write_metrics_scrape_report(request.out_dir, &report)?;
    let readback = read_metrics_scrape_report(&report_path)?;
    if readback != report {
        return Err(PolyError::diagnostics(
            ERR_METRICS_READBACK_MISMATCH,
            format!(
                "metrics scrape report changed during readback from {}",
                report_path.display()
            ),
        ));
    }
    Ok(MetricsScrapeRun {
        report_path,
        report: readback,
    })
}

pub fn compute_metrics_scrape_report(
    request: &MetricsScrapeRequest<'_>,
) -> Result<MetricsScrapeReport> {
    validate_request(request)?;
    let bytes = std::fs::read(request.metrics_path).map_err(|err| {
        PolyError::diagnostics(
            ERR_METRICS_MISSING_SINK,
            format!(
                "read metrics sink {}: {err}",
                request.metrics_path.display()
            ),
        )
    })?;
    let text = std::str::from_utf8(&bytes).map_err(|err| {
        PolyError::diagnostics(
            ERR_METRICS_MALFORMED,
            format!(
                "metrics sink {} is not UTF-8: {err}",
                request.metrics_path.display()
            ),
        )
    })?;
    let samples = parse_prometheus_text(text)?;
    let checks = evaluate_checks(&samples, request.thresholds)?;
    let alert_count = checks.iter().filter(|check| !check.passed).count();
    Ok(MetricsScrapeReport {
        schema_version: METRICS_SCRAPE_SCHEMA_VERSION.to_string(),
        artifact_kind: METRICS_SCRAPE_ARTIFACT_KIND.to_string(),
        source: request.source.to_string(),
        metrics_path: request.metrics_path.display().to_string(),
        metrics_blake3: blake3::hash(&bytes).to_hex().to_string(),
        scraped_at_millis: request.scraped_at_millis,
        evaluated_at_millis: request.evaluated_at_millis,
        max_age_millis: request.max_age_millis,
        thresholds: request.thresholds,
        sample_count: samples.len(),
        samples,
        checks,
        alert_count,
        status: if alert_count == 0 {
            MetricsStatus::Healthy
        } else {
            MetricsStatus::Alert
        },
    })
}

pub fn require_metrics_scrape_healthy(report: &MetricsScrapeReport) -> Result<()> {
    if report.status == MetricsStatus::Healthy {
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
        format!("calyxd metrics alerts fired: {failed}"),
    ))
}

pub fn write_metrics_scrape_report(dir: &Path, report: &MetricsScrapeReport) -> Result<PathBuf> {
    write_json(dir, METRICS_SCRAPE_REPORT_FILE, report)
}

pub fn read_metrics_scrape_report(path: &Path) -> Result<MetricsScrapeReport> {
    read_json(path)
}

pub fn read_prometheus_samples(path: &Path) -> Result<Vec<MetricSample>> {
    let bytes = std::fs::read(path).map_err(|err| {
        PolyError::diagnostics(
            ERR_METRICS_MISSING_SINK,
            format!("read metrics sink {}: {err}", path.display()),
        )
    })?;
    let text = std::str::from_utf8(&bytes).map_err(|err| {
        PolyError::diagnostics(
            ERR_METRICS_MALFORMED,
            format!("metrics sink {} is not UTF-8: {err}", path.display()),
        )
    })?;
    parse_prometheus_samples(text)
}

pub fn parse_prometheus_samples(text: &str) -> Result<Vec<MetricSample>> {
    parse_prometheus_text(text)
}

fn validate_request(request: &MetricsScrapeRequest<'_>) -> Result<()> {
    if request.source.trim().is_empty() {
        return Err(PolyError::diagnostics(
            ERR_METRICS_INVALID_REQUEST,
            "metrics source label is required",
        ));
    }
    if request.max_age_millis == 0 {
        return Err(PolyError::diagnostics(
            ERR_METRICS_INVALID_REQUEST,
            "max_age_millis must be greater than zero",
        ));
    }
    if request.evaluated_at_millis < request.scraped_at_millis {
        return Err(PolyError::diagnostics(
            ERR_METRICS_INVALID_REQUEST,
            "evaluated_at_millis cannot be before scraped_at_millis",
        ));
    }
    if request.evaluated_at_millis - request.scraped_at_millis > request.max_age_millis {
        return Err(PolyError::diagnostics(
            ERR_METRICS_STALE,
            format!(
                "metrics scrape age {}ms exceeds max {}ms",
                request.evaluated_at_millis - request.scraped_at_millis,
                request.max_age_millis
            ),
        ));
    }
    validate_thresholds(request.thresholds)
}

fn validate_thresholds(thresholds: MetricsThresholds) -> Result<()> {
    let bounded = thresholds.min_kernel_recall_ratio.is_finite()
        && (0.0..=1.0).contains(&thresholds.min_kernel_recall_ratio)
        && thresholds.max_guard_far_ratio.is_finite()
        && (0.0..=1.0).contains(&thresholds.max_guard_far_ratio)
        && thresholds.min_n_eff.is_finite()
        && thresholds.min_n_eff > 0.0;
    if bounded {
        Ok(())
    } else {
        Err(PolyError::diagnostics(
            ERR_METRICS_INVALID_REQUEST,
            "metrics thresholds must be finite and in policy range",
        ))
    }
}

fn parse_prometheus_text(text: &str) -> Result<Vec<MetricSample>> {
    let mut samples = Vec::new();
    for (idx, raw) in text.lines().enumerate() {
        let line_number = idx + 1;
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let split = line
            .find(char::is_whitespace)
            .ok_or_else(|| malformed(line_number, "metric line must contain a sample value"))?;
        let head = &line[..split];
        let rest = line[split..].trim();
        let value_token = rest
            .split_whitespace()
            .next()
            .ok_or_else(|| malformed(line_number, "metric line must contain a sample value"))?;
        let value = value_token
            .parse::<f64>()
            .map_err(|err| malformed(line_number, format!("parse metric value: {err}")))?;
        if !value.is_finite() {
            return Err(malformed(line_number, "metric value must be finite"));
        }
        let (name, labels) = parse_metric_head(head, line_number)?;
        samples.push(MetricSample {
            name,
            labels,
            value,
            line_number,
        });
    }
    if samples.is_empty() {
        return Err(PolyError::diagnostics(
            ERR_METRICS_MALFORMED,
            "metrics scrape contained no sample lines",
        ));
    }
    Ok(samples)
}

fn parse_metric_head(head: &str, line_number: usize) -> Result<(String, Option<String>)> {
    if let Some(open) = head.find('{') {
        if !head.ends_with('}') || open == 0 || head[..open].contains('}') {
            return Err(malformed(line_number, "malformed metric label block"));
        }
        let name = &head[..open];
        validate_metric_name(name, line_number)?;
        let labels = &head[(open + 1)..(head.len() - 1)];
        if labels.trim().is_empty() || labels.contains('{') || labels.contains('}') {
            return Err(malformed(line_number, "malformed metric label block"));
        }
        Ok((name.to_string(), Some(labels.to_string())))
    } else {
        if head.contains('}') {
            return Err(malformed(line_number, "malformed metric label block"));
        }
        validate_metric_name(head, line_number)?;
        Ok((head.to_string(), None))
    }
}

fn validate_metric_name(name: &str, line_number: usize) -> Result<()> {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return Err(malformed(line_number, "metric name is empty"));
    };
    if !(first.is_ascii_alphabetic() || first == '_' || first == ':') {
        return Err(malformed(
            line_number,
            "metric name has invalid first character",
        ));
    }
    if chars.any(|ch| !(ch.is_ascii_alphanumeric() || ch == '_' || ch == ':')) {
        return Err(malformed(line_number, "metric name has invalid character"));
    }
    Ok(())
}

fn evaluate_checks(
    samples: &[MetricSample],
    thresholds: MetricsThresholds,
) -> Result<Vec<MetricCheck>> {
    let mut checks = Vec::new();
    push_min_checks(
        &mut checks,
        samples,
        METRIC_KERNEL_RECALL,
        thresholds.min_kernel_recall_ratio,
    )?;
    push_max_checks(
        &mut checks,
        samples,
        METRIC_GUARD_FAR,
        thresholds.max_guard_far_ratio,
    )?;
    push_min_checks(
        &mut checks,
        samples,
        METRIC_PANEL_N_EFF,
        thresholds.min_n_eff,
    )?;
    push_exact_checks(&mut checks, samples, METRIC_CHAIN_VERIFY_PASSED, 1.0)?;
    Ok(checks)
}

fn push_min_checks(
    checks: &mut Vec<MetricCheck>,
    samples: &[MetricSample],
    metric: &str,
    threshold: f64,
) -> Result<()> {
    let selected = matching_samples(samples, metric)?;
    for sample in selected {
        let passed = sample.value >= threshold;
        checks.push(check(metric, sample, ">=", threshold, passed));
    }
    Ok(())
}

fn push_max_checks(
    checks: &mut Vec<MetricCheck>,
    samples: &[MetricSample],
    metric: &str,
    threshold: f64,
) -> Result<()> {
    let selected = matching_samples(samples, metric)?;
    for sample in selected {
        let passed = sample.value <= threshold;
        checks.push(check(metric, sample, "<=", threshold, passed));
    }
    Ok(())
}

fn push_exact_checks(
    checks: &mut Vec<MetricCheck>,
    samples: &[MetricSample],
    metric: &str,
    threshold: f64,
) -> Result<()> {
    let selected = matching_samples(samples, metric)?;
    for sample in selected {
        let passed = (sample.value - threshold).abs() <= f64::EPSILON;
        checks.push(check(metric, sample, "==", threshold, passed));
    }
    Ok(())
}

fn matching_samples<'a>(
    samples: &'a [MetricSample],
    metric: &str,
) -> Result<Vec<&'a MetricSample>> {
    let selected = samples
        .iter()
        .filter(|sample| sample.name == metric)
        .collect::<Vec<_>>();
    if selected.is_empty() {
        Err(PolyError::diagnostics(
            ERR_METRICS_MISSING_REQUIRED,
            format!("required metrics family {metric} was absent from scrape"),
        ))
    } else {
        Ok(selected)
    }
}

fn check(
    metric: &str,
    sample: &MetricSample,
    comparator: &str,
    threshold: f64,
    passed: bool,
) -> MetricCheck {
    let labels = sample
        .labels
        .as_deref()
        .map_or(String::new(), |labels| format!("{{{labels}}}"));
    let message = if passed {
        format!(
            "{metric}{labels}={} passed {comparator} {threshold}",
            sample.value
        )
    } else {
        format!(
            "{metric}{labels}={} failed {comparator} {threshold}",
            sample.value
        )
    };
    MetricCheck {
        metric: metric.to_string(),
        labels: sample.labels.clone(),
        value: sample.value,
        comparator: comparator.to_string(),
        threshold,
        passed,
        code: if passed {
            "CALYX_POLY_METRICS_CHECK_PASSED".to_string()
        } else {
            ERR_METRICS_ALERT.to_string()
        },
        message,
    }
}

fn malformed(line_number: usize, message: impl Into<String>) -> PolyError {
    PolyError::diagnostics(
        ERR_METRICS_MALFORMED,
        format!("line {line_number}: {}", message.into()),
    )
}
