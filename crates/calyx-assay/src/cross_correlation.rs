//! Signed cross-correlation function over explicit lags (#62).
//!
//! Lag convention: a positive lag `k` correlates `x[t]` with `y[t+k]`, so a
//! positive peak means X leads Y by `k` samples. A negative peak means Y leads X.
//! Each lag is a real Pearson correlation over the shifted paired samples.

use calyx_core::{CalyxError, Result};
use serde::{Deserialize, Serialize};

use crate::partial_correlation::{MIN_PEARSON_SAMPLES, pearson};

pub const CCF_LAG_CONVENTION: &str =
    "positive lag k correlates x[t] with y[t+k], so X leads Y by k samples";

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct CrossCorrelationPoint {
    pub lag: isize,
    pub correlation: f32,
    pub p_value: f32,
    pub ci_low: f32,
    pub ci_high: f32,
    pub n_pairs: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CrossCorrelationReport {
    pub lag_convention: String,
    pub max_lag: usize,
    pub n_samples: usize,
    pub peak_lag: isize,
    pub peak_correlation: f32,
    pub peak_abs_correlation: f32,
    pub points: Vec<CrossCorrelationPoint>,
}

pub fn cross_correlation_profile(
    x: &[f32],
    y: &[f32],
    max_lag: usize,
) -> Result<CrossCorrelationReport> {
    if x.len() != y.len() {
        return Err(CalyxError::assay_insufficient_samples(format!(
            "CCF requires paired series: x={} y={}",
            x.len(),
            y.len()
        )));
    }
    let n = x.len();
    if n < MIN_PEARSON_SAMPLES {
        return Err(CalyxError::assay_insufficient_samples(format!(
            "CCF requires at least {MIN_PEARSON_SAMPLES} paired samples; got {n}"
        )));
    }
    if max_lag > n - MIN_PEARSON_SAMPLES {
        return Err(CalyxError::assay_insufficient_samples(format!(
            "CCF max_lag {max_lag} leaves fewer than {MIN_PEARSON_SAMPLES} paired samples at the boundary for n={n}"
        )));
    }

    let mut points = Vec::with_capacity(max_lag * 2 + 1);
    for lag in -(max_lag as isize)..=(max_lag as isize) {
        let (xs, ys) = shifted_pairs(x, y, lag);
        let r = pearson(&xs, &ys)?;
        points.push(CrossCorrelationPoint {
            lag,
            correlation: r.r,
            p_value: r.p_value,
            ci_low: r.ci_low,
            ci_high: r.ci_high,
            n_pairs: r.n_samples,
        });
    }

    let peak = points
        .iter()
        .max_by(|left, right| {
            let by_abs = left.correlation.abs().total_cmp(&right.correlation.abs());
            by_abs
                .then_with(|| left.n_pairs.cmp(&right.n_pairs))
                .then_with(|| right.lag.abs().cmp(&left.lag.abs()))
        })
        .expect("non-empty lag range");

    Ok(CrossCorrelationReport {
        lag_convention: CCF_LAG_CONVENTION.to_string(),
        max_lag,
        n_samples: n,
        peak_lag: peak.lag,
        peak_correlation: peak.correlation,
        peak_abs_correlation: peak.correlation.abs(),
        points,
    })
}

fn shifted_pairs(x: &[f32], y: &[f32], lag: isize) -> (Vec<f32>, Vec<f32>) {
    let n = x.len();
    if lag >= 0 {
        let shift = lag as usize;
        (x[..n - shift].to_vec(), y[shift..].to_vec())
    } else {
        let shift = lag.unsigned_abs();
        (x[shift..].to_vec(), y[..n - shift].to_vec())
    }
}
