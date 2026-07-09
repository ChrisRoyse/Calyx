//! Pearson correlation with exact inference, and **partial correlation** — the
//! de-confounding estimator that answers "is X still associated with the outcome
//! Y once the whole-market move (and any other confounder) is held fixed?" (#58).
//!
//! A raw Pearson `r_xy` is confounded whenever a third signal `Z` drives both X
//! and Y: a basket of tickers all rise together on a market-wide move, inflating
//! every pairwise correlation. The partial correlation `r_xy·Z` removes the
//! linear contribution of the control(s) from both X and Y and correlates the
//! residuals, isolating the *direct* association.
//!
//! Two independent derivations are provided and cross-checked against each other
//! in the tests:
//! - [`partial_correlation`] — the closed-form **first-order** formula
//!   `r_xy·z = (r_xy − r_xz·r_yz) / √((1−r_xz²)(1−r_yz²))`, controlling for a
//!   single `Z`.
//! - [`partial_correlation_controlling`] — the general **precision-matrix**
//!   estimator (partial corr = `−P_xy/√(P_xx·P_yy)` where `P = R⁻¹` is the
//!   inverse of the correlation matrix over `[X, Y, controls…]`), which controls
//!   for an arbitrary set of confounders simultaneously.
//!
//! Significance is the exact Student-t statistic `t = r·√(df/(1−r²))` on
//! `df = n − 2 − k` (k = number of controls; k = 0 for plain Pearson). The 95%
//! confidence interval uses the Fisher z-transform with the partial-correlation
//! standard error `1/√(n − 3 − k)`.
//!
//! Everything fails closed: length mismatch, non-finite values, too few samples,
//! a constant (zero-variance) column, or a singular correlation matrix (collinear
//! controls) returns a structured `CalyxError`, never a silent `NaN`.

use calyx_core::{CalyxError, Result};
use serde::{Deserialize, Serialize};

use crate::special_fn::student_t_two_sided_p;

/// Minimum samples for a defined zero-order Pearson test (`df = n − 2 ≥ 1`).
pub const MIN_PEARSON_SAMPLES: usize = 3;

/// Two-sided standard-normal 95% quantile, for the Fisher-z confidence interval.
const Z_95: f64 = 1.959_963_984_540_054;

/// Residual-variance floor for the first-order partial correlation. If a control
/// explains all but this fraction of X's (or Y's) variance — `1 − r² < ε` — the
/// residual is numerically indistinguishable from a constant and the partial is
/// undefined. Set well above machine epsilon so near-collinear controls (whose
/// `1 − r²` is only ~1e-10 from float rounding on exactly-collinear integer data)
/// fail closed rather than producing a garbage `0/0` ratio.
const MIN_RESIDUAL_VARIANCE_FRACTION: f64 = 1e-9;

/// Pearson (zero-order) correlation with exact-t significance and a Fisher-z 95%
/// confidence interval.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct PearsonReport {
    pub r: f32,
    pub t_statistic: f32,
    pub p_value: f32,
    pub ci_low: f32,
    pub ci_high: f32,
    pub n_samples: usize,
}

/// Partial correlation of `X` and `Y` controlling for `k` confounders, with the
/// raw (zero-order) `r_xy` retained so callers can see how much of the apparent
/// association was confounded.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct PartialReport {
    /// Partial correlation `r_xy·controls`.
    pub partial_r: f32,
    /// Raw Pearson `r_xy` before controlling (for the de-confounding delta).
    pub zero_order_r: f32,
    pub t_statistic: f32,
    pub p_value: f32,
    pub ci_low: f32,
    pub ci_high: f32,
    /// Number of controlled confounders `k`.
    pub n_controls: usize,
    pub n_samples: usize,
}

/// Pearson correlation over paired samples with exact-t inference.
pub fn pearson(x: &[f32], y: &[f32]) -> Result<PearsonReport> {
    if x.len() != y.len() {
        return Err(CalyxError::assay_insufficient_samples(format!(
            "Pearson requires paired samples: x={} y={}",
            x.len(),
            y.len()
        )));
    }
    let n = x.len();
    if n < MIN_PEARSON_SAMPLES {
        return Err(CalyxError::assay_insufficient_samples(format!(
            "Pearson requires at least {MIN_PEARSON_SAMPLES} paired samples; got {n}"
        )));
    }
    let xd = to_finite_f64("Pearson", "x", x)?;
    let yd = to_finite_f64("Pearson", "y", y)?;
    let r = pearson_r(&xd, &yd).ok_or_else(|| {
        CalyxError::assay_degenerate_input(
            "Pearson undefined: a column is constant (zero variance)",
        )
    })?;
    let (t_statistic, p_value, ci_low, ci_high) = correlation_inference(r, n, 0)?;
    Ok(PearsonReport {
        r: r as f32,
        t_statistic: t_statistic as f32,
        p_value: p_value as f32,
        ci_low: ci_low as f32,
        ci_high: ci_high as f32,
        n_samples: n,
    })
}

/// First-order partial correlation of `X` and `Y` controlling for a single `Z`,
/// via the closed-form residual formula.
pub fn partial_correlation(x: &[f32], y: &[f32], z: &[f32]) -> Result<PartialReport> {
    if x.len() != y.len() || x.len() != z.len() {
        return Err(CalyxError::assay_insufficient_samples(format!(
            "partial correlation requires equal-length x/y/z: x={} y={} z={}",
            x.len(),
            y.len(),
            z.len()
        )));
    }
    let n = x.len();
    // df = n − 2 − 1 ≥ 1  ⇒  n ≥ 4.
    if n < MIN_PEARSON_SAMPLES + 1 {
        return Err(CalyxError::assay_insufficient_samples(format!(
            "first-order partial correlation requires at least {} samples; got {n}",
            MIN_PEARSON_SAMPLES + 1
        )));
    }
    let xd = to_finite_f64("partial correlation", "x", x)?;
    let yd = to_finite_f64("partial correlation", "y", y)?;
    let zd = to_finite_f64("partial correlation", "z", z)?;

    let degenerate = || {
        CalyxError::assay_degenerate_input(
            "partial correlation undefined: a column is constant (zero variance)",
        )
    };
    let rxy = pearson_r(&xd, &yd).ok_or_else(degenerate)?;
    let rxz = pearson_r(&xd, &zd).ok_or_else(degenerate)?;
    let ryz = pearson_r(&yd, &zd).ok_or_else(degenerate)?;

    // Denominator vanishes iff X (or Y) is perfectly explained by Z: the residual
    // has (near-)zero variance, so the partial correlation is genuinely undefined.
    let res_x = 1.0 - rxz * rxz;
    let res_y = 1.0 - ryz * ryz;
    if res_x < MIN_RESIDUAL_VARIANCE_FRACTION || res_y < MIN_RESIDUAL_VARIANCE_FRACTION {
        return Err(CalyxError::assay_degenerate_input(
            "partial correlation undefined: a control (near-)perfectly explains X or Y (zero residual variance)",
        ));
    }
    let denom = (res_x * res_y).sqrt();
    let partial = ((rxy - rxz * ryz) / denom).clamp(-1.0, 1.0);
    let (t_statistic, p_value, ci_low, ci_high) = correlation_inference(partial, n, 1)?;
    Ok(PartialReport {
        partial_r: partial as f32,
        zero_order_r: rxy as f32,
        t_statistic: t_statistic as f32,
        p_value: p_value as f32,
        ci_low: ci_low as f32,
        ci_high: ci_high as f32,
        n_controls: 1,
        n_samples: n,
    })
}

/// General partial correlation of `X` and `Y` controlling for every column in
/// `controls` simultaneously, via the inverse (precision) of the correlation
/// matrix over `[X, Y, controls…]`.
pub fn partial_correlation_controlling(
    x: &[f32],
    y: &[f32],
    controls: &[&[f32]],
) -> Result<PartialReport> {
    if controls.is_empty() {
        return Err(CalyxError::assay_insufficient_samples(
            "partial correlation requires at least one control column; use `pearson` for zero-order",
        ));
    }
    let k = controls.len();
    let n = x.len();
    if y.len() != n || controls.iter().any(|c| c.len() != n) {
        return Err(CalyxError::assay_insufficient_samples(format!(
            "partial correlation requires all columns length {n}: y={}, controls={:?}",
            y.len(),
            controls.iter().map(|c| c.len()).collect::<Vec<_>>()
        )));
    }
    // df = n − 2 − k ≥ 1.
    if n < k + MIN_PEARSON_SAMPLES {
        return Err(CalyxError::assay_insufficient_samples(format!(
            "partial correlation controlling for {k} confounders requires at least {} samples; got {n}",
            k + MIN_PEARSON_SAMPLES
        )));
    }

    // Column-major variable table: v[0]=X, v[1]=Y, v[2..]=controls. All finite.
    let mut vars: Vec<Vec<f64>> = Vec::with_capacity(2 + k);
    vars.push(to_finite_f64("partial correlation", "x", x)?);
    vars.push(to_finite_f64("partial correlation", "y", y)?);
    for (i, c) in controls.iter().enumerate() {
        vars.push(to_finite_f64(
            "partial correlation",
            &format!("control[{i}]"),
            c,
        )?);
    }

    let d = vars.len();
    let degenerate = || {
        CalyxError::assay_degenerate_input(
            "partial correlation undefined: a column is constant (zero variance)",
        )
    };
    // Correlation matrix R (symmetric, unit diagonal).
    let mut r_mat = vec![0.0f64; d * d];
    for i in 0..d {
        r_mat[i * d + i] = 1.0;
        for j in (i + 1)..d {
            let rij = pearson_r(&vars[i], &vars[j]).ok_or_else(degenerate)?;
            r_mat[i * d + j] = rij;
            r_mat[j * d + i] = rij;
        }
    }
    // Precision matrix P = R⁻¹. Singularity ⇒ collinear controls ⇒ undefined.
    let p = invert_symmetric(&r_mat, d).ok_or_else(|| {
        CalyxError::assay_degenerate_input(
            "partial correlation undefined: correlation matrix is singular (collinear columns)",
        )
    })?;
    partial_report_from_precision(r_mat[1], p[1], p[0], p[d + 1], n, k)
}

// ----- shared numerics -------------------------------------------------------

/// Exact-t significance and Fisher-z 95% CI for a (partial) correlation `r` with
/// `k` controls on `n` samples. `df = n − 2 − k`; Fisher SE = `1/√(n − 3 − k)`.
pub(crate) fn partial_report_from_precision(
    zero_order: f64,
    pxy: f64,
    pxx: f64,
    pyy: f64,
    n: usize,
    k: usize,
) -> Result<PartialReport> {
    let denom = (pxx * pyy).sqrt();
    if denom.is_nan() || denom <= 0.0 {
        return Err(CalyxError::assay_degenerate_input(
            "partial correlation undefined: non-positive precision diagonal (residual variance vanished)",
        ));
    }
    let partial = (-pxy / denom).clamp(-1.0, 1.0);
    let (t_statistic, p_value, ci_low, ci_high) = correlation_inference(partial, n, k)?;
    Ok(PartialReport {
        partial_r: partial as f32,
        zero_order_r: zero_order as f32,
        t_statistic: t_statistic as f32,
        p_value: p_value as f32,
        ci_low: ci_low as f32,
        ci_high: ci_high as f32,
        n_controls: k,
        n_samples: n,
    })
}

pub(crate) fn correlation_inference(r: f64, n: usize, k: usize) -> Result<(f64, f64, f64, f64)> {
    let df = (n as f64) - 2.0 - k as f64;
    debug_assert!(df >= 1.0, "callers guarantee df ≥ 1");
    let one_minus = 1.0 - r * r;
    let (t_statistic, p_value) = if one_minus <= f64::EPSILON {
        // Perfect (partial) linear relationship: t → ∞, maximal significance.
        (r.signum() * f64::INFINITY, 0.0)
    } else {
        let t = r * (df / one_minus).sqrt();
        (t, student_t_two_sided_p(t, df)?)
    };
    // Fisher z-transform CI. The partial-correlation SE loses one df per control.
    let se_df = (n as f64) - 3.0 - k as f64;
    let (ci_low, ci_high) = if se_df > 0.0 && one_minus > f64::EPSILON {
        let z = atanh(r);
        let se = (1.0 / se_df).sqrt();
        (
            (z - Z_95 * se).tanh().clamp(-1.0, 1.0),
            (z + Z_95 * se).tanh().clamp(-1.0, 1.0),
        )
    } else {
        // SE undefined (n = k + 3) or perfect r: collapse the CI to the point.
        (r.clamp(-1.0, 1.0), r.clamp(-1.0, 1.0))
    };
    Ok((t_statistic, p_value, ci_low, ci_high))
}

pub(crate) fn to_finite_f64(what: &str, name: &str, values: &[f32]) -> Result<Vec<f64>> {
    let mut out = Vec::with_capacity(values.len());
    for (idx, &v) in values.iter().enumerate() {
        if !v.is_finite() {
            return Err(CalyxError::assay_insufficient_samples(format!(
                "{what} {name}[{idx}] is not finite ({v})"
            )));
        }
        out.push(v as f64);
    }
    Ok(out)
}

/// Pearson correlation of two equal-length vectors; `None` if either is constant
/// (zero variance → correlation undefined).
pub(crate) fn pearson_r(x: &[f64], y: &[f64]) -> Option<f64> {
    let n = x.len() as f64;
    let mx = x.iter().sum::<f64>() / n;
    let my = y.iter().sum::<f64>() / n;
    let mut cov = 0.0;
    let mut vx = 0.0;
    let mut vy = 0.0;
    for (&a, &b) in x.iter().zip(y) {
        let da = a - mx;
        let db = b - my;
        cov += da * db;
        vx += da * da;
        vy += db * db;
    }
    if vx <= 0.0 || vy <= 0.0 {
        return None;
    }
    Some((cov / (vx.sqrt() * vy.sqrt())).clamp(-1.0, 1.0))
}

/// Invert a `d×d` (symmetric, well-conditioned) matrix by Gauss–Jordan with
/// partial pivoting. Returns `None` if the matrix is singular to working
/// precision — the caller maps that to a fail-closed degenerate error.
pub(crate) fn invert_symmetric(m: &[f64], d: usize) -> Option<Vec<f64>> {
    // Augmented [M | I].
    let mut a = vec![0.0f64; d * 2 * d];
    for i in 0..d {
        for j in 0..d {
            a[i * 2 * d + j] = m[i * d + j];
        }
        a[i * 2 * d + d + i] = 1.0;
    }
    for col in 0..d {
        // Partial pivot: largest magnitude in this column at/below the diagonal.
        let mut pivot = col;
        let mut best = a[col * 2 * d + col].abs();
        for row in (col + 1)..d {
            let v = a[row * 2 * d + col].abs();
            if v > best {
                best = v;
                pivot = row;
            }
        }
        if best < 1e-12 {
            return None; // singular
        }
        if pivot != col {
            for j in 0..(2 * d) {
                a.swap(col * 2 * d + j, pivot * 2 * d + j);
            }
        }
        let inv_p = 1.0 / a[col * 2 * d + col];
        for j in 0..(2 * d) {
            a[col * 2 * d + j] *= inv_p;
        }
        for row in 0..d {
            if row == col {
                continue;
            }
            let factor = a[row * 2 * d + col];
            if factor == 0.0 {
                continue;
            }
            for j in 0..(2 * d) {
                a[row * 2 * d + j] -= factor * a[col * 2 * d + j];
            }
        }
    }
    let mut inv = vec![0.0f64; d * d];
    for i in 0..d {
        for j in 0..d {
            inv[i * d + j] = a[i * 2 * d + d + j];
        }
    }
    Some(inv)
}

fn atanh(r: f64) -> f64 {
    0.5 * ((1.0 + r) / (1.0 - r)).ln()
}

#[cfg(test)]
#[path = "partial_correlation_tests.rs"]
mod tests;
