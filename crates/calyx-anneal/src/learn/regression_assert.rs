use calyx_core::{AnchorKind, CalyxError, Constellation, CxId, Result, VaultStore};
use serde::{Deserialize, Serialize};

use super::{MistakeLog, MistakeRef, MistakeStorage, ReplayEntry};

pub const DEFAULT_MAX_REGRESSION_RATE: f64 = 0.05;
pub const CALYX_ANNEAL_REGRESSION_RECURRED: &str = "CALYX_ANNEAL_REGRESSION_RECURRED";
pub const CALYX_ANNEAL_REGRESSION_INVALID_CONFIG: &str = "CALYX_ANNEAL_REGRESSION_INVALID_CONFIG";
pub const CALYX_ANNEAL_REGRESSION_SOURCE_UNAVAILABLE: &str =
    "CALYX_ANNEAL_REGRESSION_SOURCE_UNAVAILABLE";
pub const CALYX_ANNEAL_REGRESSION_NAN_PREDICTION: &str = "CALYX_ANNEAL_REGRESSION_NAN_PREDICTION";

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct RegressionConfig {
    pub max_regression_rate: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RegressionResult {
    pub cx_id: CxId,
    pub old_prediction: f64,
    pub observed: f64,
    pub old_surprise: f64,
    pub new_prediction: f64,
    pub new_surprise: f64,
    pub recurred: bool,
    pub anchor: AnchorKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prediction_error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RegressionReport {
    pub results: Vec<RegressionResult>,
    pub regression_count: usize,
    pub passed: bool,
}

pub trait RegressionPredictor {
    fn predict_regression(&self, cx: &Constellation) -> f64;
}

pub trait RegressionContextSource {
    fn regression_constellation(&self, cx_id: CxId) -> Result<Constellation>;
}

impl<T> RegressionContextSource for T
where
    T: VaultStore,
{
    fn regression_constellation(&self, cx_id: CxId) -> Result<Constellation> {
        self.get(cx_id, self.snapshot())
    }
}

impl RegressionConfig {
    pub const fn new(max_regression_rate: f64) -> Self {
        Self {
            max_regression_rate,
        }
    }

    pub const fn strict() -> Self {
        Self::new(0.0)
    }

    pub fn validate(self) -> Result<Self> {
        if !self.max_regression_rate.is_finite() || !(0.0..=1.0).contains(&self.max_regression_rate)
        {
            return Err(CalyxError {
                code: CALYX_ANNEAL_REGRESSION_INVALID_CONFIG,
                message: "max_regression_rate must be finite and within [0, 1]".to_string(),
                remediation: "configure a bounded regression tolerance",
            });
        }
        Ok(self)
    }

    pub fn exceeds(self, report: &RegressionReport) -> Result<bool> {
        Ok(regression_rate(report)? > self.validate()?.max_regression_rate)
    }
}

impl Default for RegressionConfig {
    fn default() -> Self {
        Self::new(DEFAULT_MAX_REGRESSION_RATE)
    }
}

impl RegressionReport {
    pub fn new(results: Vec<RegressionResult>) -> Self {
        let regression_count = results.iter().filter(|result| result.recurred).count();
        Self {
            results,
            regression_count,
            passed: regression_count == 0,
        }
    }

    pub fn empty() -> Self {
        Self::new(Vec::new())
    }

    pub fn regression_rate(&self) -> Result<f64> {
        regression_rate(self)
    }
}

pub fn assert_no_regression<P, C, S>(
    heads: &P,
    batch: &[ReplayEntry],
    log: &MistakeLog<S>,
    contexts: &C,
) -> Result<RegressionReport>
where
    P: RegressionPredictor,
    C: RegressionContextSource,
    S: MistakeStorage,
{
    let mut results = Vec::with_capacity(batch.len());
    for entry in batch {
        let mistake = log.get(entry.mistake_ref.seq)?.ok_or_else(|| {
            source_unavailable(
                entry.cx_id,
                format!("missing mistake seq {}", entry.mistake_ref.seq),
            )
        })?;
        let cx = contexts
            .regression_constellation(entry.cx_id)
            .map_err(|error| source_unavailable(entry.cx_id, error.to_string()))?;
        let raw_prediction = heads.predict_regression(&cx);
        let (new_prediction, new_surprise, prediction_error) = if raw_prediction.is_finite() {
            (
                raw_prediction,
                (raw_prediction - mistake.observed).abs(),
                None,
            )
        } else {
            (
                f64::MAX,
                f64::MAX,
                Some(CALYX_ANNEAL_REGRESSION_NAN_PREDICTION.to_string()),
            )
        };
        results.push(RegressionResult {
            cx_id: entry.cx_id,
            old_prediction: mistake.predicted,
            observed: mistake.observed,
            old_surprise: mistake.surprise,
            new_prediction,
            new_surprise,
            recurred: new_surprise >= mistake.surprise,
            anchor: mistake.anchor,
            prediction_error,
        });
    }
    Ok(RegressionReport::new(results))
}

pub fn regression_rate(report: &RegressionReport) -> Result<f64> {
    if report.results.is_empty() {
        return Ok(0.0);
    }
    if report.regression_count != report.results.iter().filter(|row| row.recurred).count() {
        return Err(CalyxError {
            code: CALYX_ANNEAL_REGRESSION_SOURCE_UNAVAILABLE,
            message: "regression report count does not match result rows".to_string(),
            remediation: "regenerate the regression report from source rows",
        });
    }
    Ok(report.regression_count as f64 / report.results.len() as f64)
}

pub fn record_regression<S>(
    result: &RegressionResult,
    log: &MistakeLog<S>,
) -> Result<Option<MistakeRef>>
where
    S: MistakeStorage,
{
    if !result.recurred {
        return Ok(None);
    }
    let surprise = boosted_surprise(result.old_surprise.max(result.new_surprise));
    let predicted = if result.new_prediction < result.observed {
        result.observed - surprise
    } else {
        result.observed + surprise
    };
    log.append(
        result.cx_id,
        predicted,
        result.observed,
        result.anchor.clone(),
    )
    .map(Some)
}

pub fn regression_recurred(report: &RegressionReport) -> CalyxError {
    let rate = report.regression_rate().unwrap_or(1.0);
    CalyxError {
        code: CALYX_ANNEAL_REGRESSION_RECURRED,
        message: format!(
            "{} of {} replayed mistakes recurred; regression_rate={rate:.6}",
            report.regression_count,
            report.results.len()
        ),
        remediation: "rollback the head update and replay recurred mistakes at higher priority",
    }
}

fn boosted_surprise(value: f64) -> f64 {
    if value.is_finite() {
        value + (value.abs() * 1.0e-12).max(f64::EPSILON)
    } else {
        f64::MAX
    }
}

fn source_unavailable(cx_id: CxId, message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: CALYX_ANNEAL_REGRESSION_SOURCE_UNAVAILABLE,
        message: format!(
            "regression source for {cx_id} unavailable: {}",
            message.into()
        ),
        remediation: "restore the mistake log and constellation source before head replay",
    }
}
