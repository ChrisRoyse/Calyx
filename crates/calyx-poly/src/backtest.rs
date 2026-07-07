//! Held-out resolved-market backtesting for Poly probability outputs.
//!
//! The harness is intentionally small and fail-closed. It accepts already-resolved held-out rows,
//! computes Brier loss and a linear probability calibration fit for both the model and Polymarket
//! aggregate probability, and refuses to produce a success report unless the model beats the market
//! aggregate on the evaluated subset.

use std::collections::HashSet;
use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::{PolyError, Result};

const EPS: f64 = 1.0e-12;

/// One resolved held-out market/outcome row for backtesting.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BacktestObservation {
    /// Market condition id or other stable market key.
    pub market_id: String,
    /// Outcome token id or stable outcome key.
    pub token_id: String,
    /// Must be true; mixed train/test inputs are rejected.
    pub held_out: bool,
    /// Last timestamp included in the training set used to create this forecast.
    pub train_cutoff_ts: u64,
    /// Forecast timestamp.
    pub forecast_ts: u64,
    /// Latest source/feature observation timestamp used by this forecast.
    pub feature_max_observed_ts: u64,
    /// Timestamp when the outcome was observed as resolved.
    pub outcome_observed_ts: u64,
    /// Must be true; unresolved markets cannot score probability quality.
    pub resolved: bool,
    /// Whether this row belongs to the evaluated subset.
    pub evaluated: bool,
    /// Key used to prevent redundant rows from double-counting the same evidence contract.
    pub redundancy_key: String,
    /// Hash of the exact feature bundle used at forecast time.
    pub feature_fingerprint: String,
    /// Model probability that this outcome wins.
    pub p_model: f64,
    /// Polymarket aggregate/implied probability for the same outcome at decision time.
    pub p_market: f64,
    /// Final resolved truth for this outcome.
    pub actual_win: bool,
}

/// Aggregate metrics for one evaluated subset.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BacktestMetrics {
    /// Number of rows in this subset.
    pub count: usize,
    /// Mean squared probability error for `p_model`.
    pub model_brier: f64,
    /// Mean squared probability error for `p_market`.
    pub market_brier: f64,
    /// `market_brier - model_brier`; positive means the model is better.
    pub brier_improvement: f64,
    /// OLS intercept for `actual_win ~ p_model`.
    pub model_calibration_intercept: f64,
    /// OLS slope for `actual_win ~ p_model`.
    pub model_calibration_slope: f64,
    /// OLS intercept for `actual_win ~ p_market`.
    pub market_calibration_intercept: f64,
    /// OLS slope for `actual_win ~ p_market`.
    pub market_calibration_slope: f64,
}

/// Persisted backtest report.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BacktestReport {
    /// Report schema version.
    pub schema_version: u32,
    /// Human-readable source-of-truth description for FSV logs.
    pub source_of_truth: String,
    /// Number of input observations inspected.
    pub input_count: usize,
    /// Number of held-out resolved observations scored.
    pub held_out_count: usize,
    /// Number of evaluated held-out observations scored.
    pub evaluated_count: usize,
    /// Reproducibility fingerprint over the exact input rows.
    pub input_fingerprint: String,
    /// Metrics over every held-out row.
    pub all: BacktestMetrics,
    /// Metrics over the evaluated subset.
    pub evaluated: BacktestMetrics,
    /// True only when evaluated model Brier is lower than evaluated market Brier.
    pub beats_market_on_evaluated_subset: bool,
}

/// Runs the fail-closed held-out backtest.
pub fn run_backtest(observations: &[BacktestObservation]) -> Result<BacktestReport> {
    validate_observations(observations)?;

    let all_rows: Vec<_> = observations.iter().collect();
    let evaluated_rows: Vec<_> = observations.iter().filter(|row| row.evaluated).collect();
    if evaluated_rows.is_empty() {
        return Err(PolyError::backtest(
            "CALYX_POLY_BACKTEST_NO_EVALUATED_SUBSET",
            "backtest has no evaluated held-out rows",
        ));
    }

    let all = metrics_for(&all_rows, "all held-out rows")?;
    let evaluated = metrics_for(&evaluated_rows, "evaluated held-out rows")?;
    let beats_market = evaluated.model_brier + EPS < evaluated.market_brier;
    if !beats_market {
        return Err(PolyError::backtest(
            "CALYX_POLY_BACKTEST_BASELINE_NOT_BEATEN",
            format!(
                "evaluated model Brier {:.12} is not below market Brier {:.12}",
                evaluated.model_brier, evaluated.market_brier
            ),
        ));
    }

    Ok(BacktestReport {
        schema_version: 1,
        source_of_truth:
            "persisted JSON report generated from held-out resolved Poly backtest observations"
                .to_string(),
        input_count: observations.len(),
        held_out_count: observations.len(),
        evaluated_count: evaluated_rows.len(),
        input_fingerprint: input_fingerprint(observations)?,
        all,
        evaluated,
        beats_market_on_evaluated_subset: true,
    })
}

/// Writes a backtest report to disk as the durable source of truth.
pub fn write_backtest_report(path: &Path, report: &BacktestReport) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| {
            PolyError::backtest(
                "CALYX_POLY_BACKTEST_REPORT_WRITE",
                format!("create report directory {}: {err}", parent.display()),
            )
        })?;
    }
    let bytes = serde_json::to_vec_pretty(report).map_err(|err| {
        PolyError::backtest(
            "CALYX_POLY_BACKTEST_REPORT_ENCODE",
            format!("encode backtest report: {err}"),
        )
    })?;
    fs::write(path, bytes).map_err(|err| {
        PolyError::backtest(
            "CALYX_POLY_BACKTEST_REPORT_WRITE",
            format!("write report {}: {err}", path.display()),
        )
    })
}

/// Reads the durable backtest report from disk.
pub fn read_backtest_report(path: &Path) -> Result<BacktestReport> {
    let bytes = fs::read(path).map_err(|err| {
        PolyError::backtest(
            "CALYX_POLY_BACKTEST_REPORT_READ",
            format!("read report {}: {err}", path.display()),
        )
    })?;
    serde_json::from_slice(&bytes).map_err(|err| {
        PolyError::backtest(
            "CALYX_POLY_BACKTEST_REPORT_DECODE",
            format!("decode report {}: {err}", path.display()),
        )
    })
}

fn validate_observations(observations: &[BacktestObservation]) -> Result<()> {
    if observations.is_empty() {
        return Err(PolyError::backtest(
            "CALYX_POLY_BACKTEST_EMPTY",
            "backtest requires at least one held-out resolved observation",
        ));
    }

    let mut keys = HashSet::new();
    let mut redundancy_keys = HashSet::new();
    for (index, row) in observations.iter().enumerate() {
        if row.market_id.trim().is_empty()
            || row.token_id.trim().is_empty()
            || row.redundancy_key.trim().is_empty()
        {
            return Err(PolyError::backtest(
                "CALYX_POLY_BACKTEST_INVALID_ID",
                format!("row {index} has an empty market_id, token_id, or redundancy_key"),
            ));
        }
        if !keys.insert((row.market_id.as_str(), row.token_id.as_str())) {
            return Err(PolyError::backtest(
                "CALYX_POLY_BACKTEST_DUPLICATE_ROW",
                format!(
                    "duplicate market/outcome row market_id={} token_id={}",
                    row.market_id, row.token_id
                ),
            ));
        }
        if !redundancy_keys.insert(row.redundancy_key.as_str()) {
            return Err(PolyError::backtest(
                "CALYX_POLY_BACKTEST_REDUNDANT_ROW",
                format!("row {index} repeats redundancy_key={}", row.redundancy_key),
            ));
        }
        if !row.held_out {
            return Err(PolyError::backtest(
                "CALYX_POLY_BACKTEST_NOT_HELD_OUT",
                format!("row {index} is not marked held_out"),
            ));
        }
        if row.train_cutoff_ts >= row.forecast_ts {
            return Err(PolyError::backtest(
                "CALYX_POLY_BACKTEST_TRAIN_CUTOFF_OVERLAP",
                format!(
                    "row {index} train_cutoff_ts {} is not before forecast_ts {}",
                    row.train_cutoff_ts, row.forecast_ts
                ),
            ));
        }
        if row.feature_max_observed_ts > row.forecast_ts {
            return Err(PolyError::backtest(
                "CALYX_POLY_BACKTEST_LOOKAHEAD_FEATURE",
                format!(
                    "row {index} feature_max_observed_ts {} is after forecast_ts {}",
                    row.feature_max_observed_ts, row.forecast_ts
                ),
            ));
        }
        if row.outcome_observed_ts <= row.forecast_ts {
            return Err(PolyError::backtest(
                "CALYX_POLY_BACKTEST_OUTCOME_NOT_AFTER_FORECAST",
                format!(
                    "row {index} outcome_observed_ts {} is not after forecast_ts {}",
                    row.outcome_observed_ts, row.forecast_ts
                ),
            ));
        }
        if !row.resolved {
            return Err(PolyError::backtest(
                "CALYX_POLY_BACKTEST_UNRESOLVED",
                format!("row {index} is not resolved"),
            ));
        }
        validate_hash(&row.feature_fingerprint, index)?;
        validate_probability(row.p_model, index, "p_model")?;
        validate_probability(row.p_market, index, "p_market")?;
    }

    Ok(())
}

fn validate_hash(value: &str, index: usize) -> Result<()> {
    if value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Ok(());
    }
    Err(PolyError::backtest(
        "CALYX_POLY_BACKTEST_INVALID_FEATURE_FINGERPRINT",
        format!("row {index} feature_fingerprint must be 64 hex characters"),
    ))
}

fn input_fingerprint(observations: &[BacktestObservation]) -> Result<String> {
    let bytes = serde_json::to_vec(observations).map_err(|err| {
        PolyError::backtest(
            "CALYX_POLY_BACKTEST_INPUT_FINGERPRINT",
            format!("encode input fingerprint: {err}"),
        )
    })?;
    Ok(blake3::hash(&bytes).to_hex().to_string())
}

fn validate_probability(value: f64, index: usize, field: &str) -> Result<()> {
    if !value.is_finite() || !(0.0..=1.0).contains(&value) {
        return Err(PolyError::backtest(
            "CALYX_POLY_BACKTEST_INVALID_PROBABILITY",
            format!("row {index} field {field} must be finite and in [0, 1], got {value}"),
        ));
    }
    Ok(())
}

fn metrics_for(rows: &[&BacktestObservation], label: &str) -> Result<BacktestMetrics> {
    if rows.len() < 2 {
        return Err(PolyError::backtest(
            "CALYX_POLY_BACKTEST_TOO_FEW_ROWS",
            format!("{label} requires at least two rows"),
        ));
    }

    let mut model_loss = 0.0;
    let mut market_loss = 0.0;
    for row in rows {
        let y = outcome_value(row.actual_win);
        model_loss += squared_error(row.p_model, y);
        market_loss += squared_error(row.p_market, y);
    }
    let n = rows.len() as f64;
    let (model_intercept, model_slope) = calibration_fit(rows, |row| row.p_model, label, "model")?;
    let (market_intercept, market_slope) =
        calibration_fit(rows, |row| row.p_market, label, "market")?;
    let model_brier = model_loss / n;
    let market_brier = market_loss / n;

    Ok(BacktestMetrics {
        count: rows.len(),
        model_brier,
        market_brier,
        brier_improvement: market_brier - model_brier,
        model_calibration_intercept: model_intercept,
        model_calibration_slope: model_slope,
        market_calibration_intercept: market_intercept,
        market_calibration_slope: market_slope,
    })
}

fn squared_error(p: f64, y: f64) -> f64 {
    let err = p - y;
    err * err
}

fn outcome_value(actual_win: bool) -> f64 {
    if actual_win { 1.0 } else { 0.0 }
}

fn calibration_fit<F>(
    rows: &[&BacktestObservation],
    probability: F,
    label: &str,
    series_name: &str,
) -> Result<(f64, f64)>
where
    F: Fn(&BacktestObservation) -> f64,
{
    let n = rows.len() as f64;
    let mean_x = rows.iter().map(|row| probability(row)).sum::<f64>() / n;
    let mean_y = rows
        .iter()
        .map(|row| outcome_value(row.actual_win))
        .sum::<f64>()
        / n;

    let mut cov = 0.0;
    let mut var = 0.0;
    for row in rows {
        let dx = probability(row) - mean_x;
        let dy = outcome_value(row.actual_win) - mean_y;
        cov += dx * dy;
        var += dx * dx;
    }
    if var <= EPS {
        return Err(PolyError::backtest(
            "CALYX_POLY_BACKTEST_ZERO_VARIANCE",
            format!("{label} has zero {series_name} probability variance"),
        ));
    }

    let slope = cov / var;
    let intercept = mean_y - slope * mean_x;
    Ok((intercept, slope))
}
