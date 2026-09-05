//! Domain×horizon calibration slope: fit + de-bias (issue #86).
//!
//! Raw model probabilities and raw market prices are systematically miscalibrated per domain and
//! horizon (politics is chronically under-confident; short-horizon crypto over-reacts). This module
//! fits a **Platt scaling** slope `p_cal = σ(a + b·logit(p_raw))` on resolved `(p_raw, outcome)`
//! history for one `(domain, horizon)` bucket, then applies it to de-bias both `p_model` and the
//! market price. The fit is deterministic gradient descent on logistic loss (fixed init `a=0, b=1`,
//! fixed iteration count) so the slope is reproducible. Fail closed below the sample floor or on a
//! single-class history (a slope needs both outcomes) or non-finite input.

use serde::{Deserialize, Serialize};

use crate::error::{PolyError, Result};
use crate::forecast::{logit, sigmoid};

/// Minimum resolved samples to fit a calibration slope.
pub const MIN_CALIBRATION_SAMPLES: usize = 30;
/// Gradient-descent iterations (fixed for reproducibility).
pub const CALIBRATION_ITERS: usize = 2000;
/// Gradient-descent learning rate.
pub const CALIBRATION_LR: f64 = 0.05;

/// Too few resolved samples to fit a slope.
pub const ERR_CAL_SAMPLES: &str = "CALYX_POLY_CALIBRATION_INSUFFICIENT_SAMPLES";
/// The resolved history is single-class (all YES or all NO) — no slope is identifiable.
pub const ERR_CAL_SINGLE_CLASS: &str = "CALYX_POLY_CALIBRATION_SINGLE_CLASS";
/// A probability in the history was not finite in `[0, 1]`.
pub const ERR_CAL_PROBABILITY: &str = "CALYX_POLY_CALIBRATION_PROBABILITY";

/// Horizon buckets by seconds-to-resolution.
pub fn horizon_bucket(secs_to_resolution: f64) -> &'static str {
    if !secs_to_resolution.is_finite() || secs_to_resolution < 0.0 {
        "unknown"
    } else if secs_to_resolution < 3_600.0 {
        "lt_1h"
    } else if secs_to_resolution < 86_400.0 {
        "1h_24h"
    } else if secs_to_resolution < 7.0 * 86_400.0 {
        "1d_7d"
    } else {
        "gt_7d"
    }
}

/// A fitted per-(domain, horizon) calibration slope with in-sample Brier before/after.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CalibrationSlope {
    /// Domain slug.
    pub domain: String,
    /// Horizon bucket.
    pub horizon_bucket: String,
    /// Platt intercept.
    pub a: f64,
    /// Platt slope on the raw logit.
    pub b: f64,
    /// Resolved samples fit.
    pub n: usize,
    /// In-sample Brier score of the raw probabilities.
    pub brier_raw: f64,
    /// In-sample Brier score after calibration.
    pub brier_calibrated: f64,
}

/// Fits a calibration slope on resolved `(p_raw, outcome)` pairs for one `(domain, horizon)` bucket.
pub fn fit_calibration_slope(
    domain: &str,
    horizon_bucket: &str,
    pairs: &[(f64, bool)],
) -> Result<CalibrationSlope> {
    if pairs.len() < MIN_CALIBRATION_SAMPLES {
        return Err(PolyError::diagnostics(
            ERR_CAL_SAMPLES,
            format!(
                "calibration for {domain}/{horizon_bucket} needs >= {MIN_CALIBRATION_SAMPLES} resolved samples, got {}",
                pairs.len()
            ),
        ));
    }
    let positives = pairs.iter().filter(|(_, y)| *y).count();
    if positives == 0 || positives == pairs.len() {
        return Err(PolyError::diagnostics(
            ERR_CAL_SINGLE_CLASS,
            format!(
                "calibration for {domain}/{horizon_bucket} history is single-class ({positives} of {} YES)",
                pairs.len()
            ),
        ));
    }
    let mut features = Vec::with_capacity(pairs.len());
    for (p, y) in pairs {
        if !p.is_finite() || !(0.0..=1.0).contains(p) {
            return Err(PolyError::diagnostics(
                ERR_CAL_PROBABILITY,
                format!("calibration history probability {p} must be finite in [0, 1]"),
            ));
        }
        features.push((logit(*p), if *y { 1.0 } else { 0.0 }));
    }

    // Deterministic gradient descent on logistic loss over p_cal = sigmoid(a + b*x).
    let (mut a, mut b) = (0.0f64, 1.0f64);
    let n = features.len() as f64;
    for _ in 0..CALIBRATION_ITERS {
        let (mut grad_a, mut grad_b) = (0.0, 0.0);
        for (x, y) in &features {
            let pred = sigmoid(a + b * x);
            let err = pred - y;
            grad_a += err;
            grad_b += err * x;
        }
        a -= CALIBRATION_LR * grad_a / n;
        b -= CALIBRATION_LR * grad_b / n;
    }

    let slope = CalibrationSlope {
        domain: domain.to_string(),
        horizon_bucket: horizon_bucket.to_string(),
        a,
        b,
        n: pairs.len(),
        brier_raw: brier(pairs.iter().map(|(p, y)| (*p, *y))),
        brier_calibrated: brier(pairs.iter().map(|(p, y)| (apply_slope(a, b, *p), *y))),
    };
    Ok(slope)
}

/// Applies a fitted slope to de-bias a raw probability (a `p_model` or a market price).
pub fn apply_calibration(slope: &CalibrationSlope, p_raw: f64) -> f64 {
    apply_slope(slope.a, slope.b, p_raw)
}

fn apply_slope(a: f64, b: f64, p_raw: f64) -> f64 {
    sigmoid(a + b * logit(p_raw.clamp(0.0, 1.0)))
}

fn brier(pairs: impl Iterator<Item = (f64, bool)>) -> f64 {
    let mut sum = 0.0;
    let mut n = 0.0;
    for (p, y) in pairs {
        let target = if y { 1.0 } else { 0.0 };
        sum += (p - target).powi(2);
        n += 1.0;
    }
    if n > 0.0 { sum / n } else { 0.0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn horizon_buckets_partition() {
        assert_eq!(horizon_bucket(600.0), "lt_1h");
        assert_eq!(horizon_bucket(3_600.0), "1h_24h");
        assert_eq!(horizon_bucket(200_000.0), "1d_7d");
        assert_eq!(horizon_bucket(1_000_000.0), "gt_7d");
        assert_eq!(horizon_bucket(-1.0), "unknown");
    }

    #[test]
    fn underconfident_history_is_corrected() {
        // Construct an under-confident forecaster: it says 0.6 when the truth resolves YES 80% of the
        // time, and 0.4 when YES 20% of the time. Calibration should stretch probabilities outward
        // and lower the Brier score.
        let mut pairs = Vec::new();
        for i in 0..100 {
            // p=0.6 group, 80% YES
            pairs.push((0.6, i % 5 != 0));
            // p=0.4 group, 20% YES
            pairs.push((0.4, i % 5 == 0));
        }
        let slope = fit_calibration_slope("crypto", "1h_24h", &pairs).unwrap();
        assert!(
            slope.b > 1.0,
            "under-confidence must be stretched (b>1): b={}",
            slope.b
        );
        assert!(
            slope.brier_calibrated < slope.brier_raw,
            "calibration must lower Brier: {} -> {}",
            slope.brier_raw,
            slope.brier_calibrated
        );
        // 0.6 raw should move up toward 0.8.
        let cal = apply_calibration(&slope, 0.6);
        assert!(cal > 0.6, "0.6 should be pushed up, got {cal}");
    }

    #[test]
    fn fails_closed_below_floor_and_single_class() {
        assert_eq!(
            fit_calibration_slope("d", "h", &[(0.5, true); 10])
                .unwrap_err()
                .code(),
            ERR_CAL_SAMPLES
        );
        assert_eq!(
            fit_calibration_slope("d", "h", &[(0.5, true); 40])
                .unwrap_err()
                .code(),
            ERR_CAL_SINGLE_CLASS
        );
    }
}
