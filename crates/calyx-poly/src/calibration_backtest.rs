//! Held-out forecast calibration backtests against resolved outcome anchors (issue #97).

use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{PolyError, Result};

pub const CALIBRATION_BACKTEST_SCHEMA_VERSION: &str = "poly.calibration_backtest.v1";
pub const CALIBRATION_BACKTEST_ARTIFACT_KIND: &str = "poly_calibration_backtest";
pub const CALIBRATION_BACKTEST_REPORT_FILE: &str = "forecast-calibration-backtest-report.json";

pub const ERR_CALIBRATION_BACKTEST_EMPTY_HOLDOUT: &str =
    "CALYX_POLY_CALIBRATION_BACKTEST_EMPTY_HOLDOUT";
pub const ERR_CALIBRATION_BACKTEST_INSUFFICIENT_HOLDOUT: &str =
    "CALYX_POLY_CALIBRATION_BACKTEST_INSUFFICIENT_HOLDOUT";
pub const ERR_CALIBRATION_BACKTEST_INVALID_REQUEST: &str =
    "CALYX_POLY_CALIBRATION_BACKTEST_INVALID_REQUEST";
pub const ERR_CALIBRATION_BACKTEST_INVALID_ROW: &str =
    "CALYX_POLY_CALIBRATION_BACKTEST_INVALID_ROW";
pub const ERR_CALIBRATION_BACKTEST_LEAKAGE: &str = "CALYX_POLY_CALIBRATION_BACKTEST_LEAKAGE";
pub const ERR_CALIBRATION_BACKTEST_MISSING_ANCHOR: &str =
    "CALYX_POLY_CALIBRATION_BACKTEST_MISSING_ANCHOR";
pub const ERR_CALIBRATION_BACKTEST_FUTURE_OUTCOME: &str =
    "CALYX_POLY_CALIBRATION_BACKTEST_FUTURE_OUTCOME";
pub const ERR_CALIBRATION_BACKTEST_READBACK_MISMATCH: &str =
    "CALYX_POLY_CALIBRATION_BACKTEST_READBACK_MISMATCH";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CalibrationBacktestObservation {
    pub forecast_id: String,
    pub market_id: String,
    pub outcome_id: String,
    pub domain: String,
    pub horizon_bucket: String,
    pub held_out: bool,
    pub train_cutoff_ts: u64,
    pub forecast_ts: u64,
    pub feature_max_observed_ts: u64,
    pub outcome_observed_ts: u64,
    pub anchor_id: String,
    pub anchor_source: String,
    pub anchor_version: u32,
    pub probability: f64,
    pub actual_win: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CalibrationBacktestRequest {
    pub as_of_ts: u64,
    pub min_held_out_rows: usize,
    pub bin_count: u32,
    pub observations: Vec<CalibrationBacktestObservation>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CalibrationBacktestBin {
    pub index: u32,
    pub count: usize,
    pub lower_inclusive: f64,
    pub upper_exclusive: f64,
    pub mean_probability: Option<f64>,
    pub observed_rate: Option<f64>,
    pub brier: Option<f64>,
    pub calibration_abs_error: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DomainHorizonCoverage {
    pub domain: String,
    pub horizon_bucket: String,
    pub count: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CalibrationBacktestReport {
    pub schema_version: String,
    pub artifact_kind: String,
    pub source_of_truth: String,
    pub as_of_ts: u64,
    pub input_count: usize,
    pub held_out_count: usize,
    pub min_held_out_rows: usize,
    pub bin_count: u32,
    pub input_fingerprint: String,
    pub brier: f64,
    pub calibration_abs_error: f64,
    pub direction_accuracy: f64,
    pub reliability_bins: Vec<CalibrationBacktestBin>,
    pub domain_horizon_coverage: Vec<DomainHorizonCoverage>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CalibrationBacktestRun {
    pub report_path: PathBuf,
    pub report: CalibrationBacktestReport,
}

pub fn run_calibration_backtest_report(
    output_root: &Path,
    request: &CalibrationBacktestRequest,
) -> Result<CalibrationBacktestRun> {
    let report = compute_calibration_backtest_report(request)?;
    let report_path = write_calibration_backtest_report(output_root, &report)?;
    let expected_bytes = serde_json::to_vec_pretty(&report).map_err(|err| {
        PolyError::backtest(
            ERR_CALIBRATION_BACKTEST_INVALID_REQUEST,
            format!("encode calibration backtest report for readback check: {err}"),
        )
    })?;
    let actual_bytes = fs::read(&report_path).map_err(|err| {
        PolyError::backtest(
            ERR_CALIBRATION_BACKTEST_INVALID_REQUEST,
            format!(
                "read calibration backtest report bytes {}: {err}",
                report_path.display()
            ),
        )
    })?;
    if actual_bytes != expected_bytes {
        return Err(PolyError::backtest(
            ERR_CALIBRATION_BACKTEST_READBACK_MISMATCH,
            format!(
                "calibration backtest report {} bytes did not read back as written",
                report_path.display()
            ),
        ));
    }
    let readback = read_calibration_backtest_report(&report_path)?;
    Ok(CalibrationBacktestRun {
        report_path,
        report: readback,
    })
}

pub fn compute_calibration_backtest_report(
    request: &CalibrationBacktestRequest,
) -> Result<CalibrationBacktestReport> {
    validate_request(request)?;
    let held_out: Vec<_> = request
        .observations
        .iter()
        .filter(|row| row.held_out)
        .collect();
    if held_out.is_empty() {
        return Err(PolyError::backtest(
            ERR_CALIBRATION_BACKTEST_EMPTY_HOLDOUT,
            "calibration backtest has no held-out forecast rows",
        ));
    }
    if held_out.len() < request.min_held_out_rows {
        return Err(PolyError::backtest(
            ERR_CALIBRATION_BACKTEST_INSUFFICIENT_HOLDOUT,
            format!(
                "calibration backtest needs >= {} held-out rows, got {}",
                request.min_held_out_rows,
                held_out.len()
            ),
        ));
    }
    validate_rows(request.as_of_ts, &held_out)?;

    let mut bin_acc = vec![BinAccumulator::default(); request.bin_count as usize];
    let mut coverage = BTreeMap::new();
    let mut brier_sum = 0.0;
    let mut correct = 0usize;
    for row in &held_out {
        let y = outcome_value(row.actual_win);
        let err = row.probability - y;
        brier_sum += err * err;
        correct += usize::from((row.probability >= 0.5) == row.actual_win);
        bin_acc[bin_index(row.probability, request.bin_count) as usize].push(row.probability, y);
        *coverage
            .entry((row.domain.clone(), row.horizon_bucket.clone()))
            .or_insert(0usize) += 1;
    }

    let n = held_out.len() as f64;
    let bins: Vec<_> = bin_acc
        .into_iter()
        .enumerate()
        .map(|(index, acc)| acc.finish(index as u32, request.bin_count))
        .collect();
    let calibration_abs_error = bins
        .iter()
        .filter_map(|bin| {
            bin.calibration_abs_error
                .map(|err| err * bin.count as f64 / n)
        })
        .sum();
    let domain_horizon_coverage = coverage
        .into_iter()
        .map(|((domain, horizon_bucket), count)| DomainHorizonCoverage {
            domain,
            horizon_bucket,
            count,
        })
        .collect();

    Ok(CalibrationBacktestReport {
        schema_version: CALIBRATION_BACKTEST_SCHEMA_VERSION.to_string(),
        artifact_kind: CALIBRATION_BACKTEST_ARTIFACT_KIND.to_string(),
        source_of_truth: "local held-out forecast rows joined to resolved outcome anchors"
            .to_string(),
        as_of_ts: request.as_of_ts,
        input_count: request.observations.len(),
        held_out_count: held_out.len(),
        min_held_out_rows: request.min_held_out_rows,
        bin_count: request.bin_count,
        input_fingerprint: input_fingerprint(request)?,
        brier: brier_sum / n,
        calibration_abs_error,
        direction_accuracy: correct as f64 / n,
        reliability_bins: bins,
        domain_horizon_coverage,
    })
}

pub fn write_calibration_backtest_report(
    dir: &Path,
    report: &CalibrationBacktestReport,
) -> Result<PathBuf> {
    fs::create_dir_all(dir).map_err(|err| {
        PolyError::backtest(
            ERR_CALIBRATION_BACKTEST_INVALID_REQUEST,
            format!(
                "create calibration backtest directory {}: {err}",
                dir.display()
            ),
        )
    })?;
    let path = dir.join(CALIBRATION_BACKTEST_REPORT_FILE);
    let bytes = serde_json::to_vec_pretty(report).map_err(|err| {
        PolyError::backtest(
            ERR_CALIBRATION_BACKTEST_INVALID_REQUEST,
            format!("encode calibration backtest report: {err}"),
        )
    })?;
    fs::write(&path, bytes).map_err(|err| {
        PolyError::backtest(
            ERR_CALIBRATION_BACKTEST_INVALID_REQUEST,
            format!(
                "write calibration backtest report {}: {err}",
                path.display()
            ),
        )
    })?;
    Ok(path)
}

pub fn read_calibration_backtest_report(path: &Path) -> Result<CalibrationBacktestReport> {
    let bytes = fs::read(path).map_err(|err| {
        PolyError::backtest(
            ERR_CALIBRATION_BACKTEST_INVALID_REQUEST,
            format!("read calibration backtest report {}: {err}", path.display()),
        )
    })?;
    serde_json::from_slice(&bytes).map_err(|err| {
        PolyError::backtest(
            ERR_CALIBRATION_BACKTEST_INVALID_REQUEST,
            format!(
                "decode calibration backtest report {}: {err}",
                path.display()
            ),
        )
    })
}

fn validate_request(request: &CalibrationBacktestRequest) -> Result<()> {
    if request.min_held_out_rows == 0 || request.bin_count == 0 || request.bin_count > 100 {
        return Err(PolyError::backtest(
            ERR_CALIBRATION_BACKTEST_INVALID_REQUEST,
            "min_held_out_rows must be positive and bin_count must be in 1..=100",
        ));
    }
    Ok(())
}

fn validate_rows(as_of_ts: u64, rows: &[&CalibrationBacktestObservation]) -> Result<()> {
    let mut keys = HashSet::new();
    for (index, row) in rows.iter().enumerate() {
        validate_label(index, "forecast_id", &row.forecast_id)?;
        validate_label(index, "market_id", &row.market_id)?;
        validate_label(index, "outcome_id", &row.outcome_id)?;
        validate_label(index, "domain", &row.domain)?;
        validate_label(index, "horizon_bucket", &row.horizon_bucket)?;
        if !keys.insert((row.forecast_id.as_str(), row.outcome_id.as_str())) {
            return Err(PolyError::backtest(
                ERR_CALIBRATION_BACKTEST_INVALID_ROW,
                format!(
                    "row {index} duplicates forecast_id={} outcome_id={}",
                    row.forecast_id, row.outcome_id
                ),
            ));
        }
        if row.anchor_id.trim().is_empty()
            || row.anchor_source.trim().is_empty()
            || row.anchor_version == 0
        {
            return Err(PolyError::backtest(
                ERR_CALIBRATION_BACKTEST_MISSING_ANCHOR,
                format!("row {index} is missing a resolved outcome anchor"),
            ));
        }
        if row.train_cutoff_ts >= row.forecast_ts
            || row.feature_max_observed_ts > row.forecast_ts
            || row.outcome_observed_ts <= row.forecast_ts
        {
            return Err(PolyError::backtest(
                ERR_CALIBRATION_BACKTEST_LEAKAGE,
                format!(
                    "row {index} violates held-out timing: train_cutoff_ts={} \
                     forecast_ts={} feature_max_observed_ts={} outcome_observed_ts={}",
                    row.train_cutoff_ts,
                    row.forecast_ts,
                    row.feature_max_observed_ts,
                    row.outcome_observed_ts
                ),
            ));
        }
        if row.outcome_observed_ts > as_of_ts {
            return Err(PolyError::backtest(
                ERR_CALIBRATION_BACKTEST_FUTURE_OUTCOME,
                format!(
                    "row {index} outcome_observed_ts {} is after backtest as_of_ts {}",
                    row.outcome_observed_ts, as_of_ts
                ),
            ));
        }
        if !row.probability.is_finite() || !(0.0..=1.0).contains(&row.probability) {
            return Err(PolyError::backtest(
                ERR_CALIBRATION_BACKTEST_INVALID_ROW,
                format!("row {index} probability must be finite in [0, 1]"),
            ));
        }
    }
    Ok(())
}

fn validate_label(index: usize, field: &str, value: &str) -> Result<()> {
    if value.trim().is_empty() {
        return Err(PolyError::backtest(
            ERR_CALIBRATION_BACKTEST_INVALID_ROW,
            format!("row {index} field {field} is required"),
        ));
    }
    Ok(())
}

fn input_fingerprint(request: &CalibrationBacktestRequest) -> Result<String> {
    let bytes = serde_json::to_vec(request).map_err(|err| {
        PolyError::backtest(
            ERR_CALIBRATION_BACKTEST_INVALID_REQUEST,
            format!("encode calibration backtest input fingerprint: {err}"),
        )
    })?;
    Ok(blake3::hash(&bytes).to_hex().to_string())
}

fn bin_index(probability: f64, count: u32) -> u32 {
    let mut index = (probability * f64::from(count)).floor() as u32;
    if index >= count {
        index = count - 1;
    }
    index
}

fn outcome_value(actual_win: bool) -> f64 {
    if actual_win { 1.0 } else { 0.0 }
}

#[derive(Clone, Default)]
struct BinAccumulator {
    count: usize,
    probability_sum: f64,
    outcome_sum: f64,
    brier_sum: f64,
}

impl BinAccumulator {
    fn push(&mut self, probability: f64, outcome: f64) {
        self.count += 1;
        self.probability_sum += probability;
        self.outcome_sum += outcome;
        self.brier_sum += (probability - outcome).powi(2);
    }

    fn finish(self, index: u32, bin_count: u32) -> CalibrationBacktestBin {
        let lower = f64::from(index) / f64::from(bin_count);
        let upper = f64::from(index + 1) / f64::from(bin_count);
        if self.count == 0 {
            return CalibrationBacktestBin {
                index,
                count: 0,
                lower_inclusive: lower,
                upper_exclusive: upper,
                mean_probability: None,
                observed_rate: None,
                brier: None,
                calibration_abs_error: None,
            };
        }
        let n = self.count as f64;
        let mean_probability = self.probability_sum / n;
        let observed_rate = self.outcome_sum / n;
        CalibrationBacktestBin {
            index,
            count: self.count,
            lower_inclusive: lower,
            upper_exclusive: upper,
            mean_probability: Some(mean_probability),
            observed_rate: Some(observed_rate),
            brier: Some(self.brier_sum / n),
            calibration_abs_error: Some((mean_probability - observed_rate).abs()),
        }
    }
}
