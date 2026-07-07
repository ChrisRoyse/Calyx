//! Linear Granger causality — does the past of `X` help predict `Y` beyond
//! `Y`'s own past? (#60). This is the *linear, parametric* complement to the
//! non-parametric [`transfer_entropy`](crate::transfer_entropy) lens: transfer
//! entropy captures arbitrary (including non-linear) directed information flow,
//! while Granger gives a fast, interpretable F-test for a linear vector
//! autoregression — cheap enough to sweep across every candidate driver.
//!
//! Construction (Granger, 1969):
//! - **Restricted** model regresses `y_t` on its own `p` lags plus an intercept:
//!   `y_t = c + Σ_{i=1..p} a_i·y_{t−i} + u_t`.
//! - **Unrestricted** model adds the `p` lags of `X`:
//!   `y_t = c + Σ a_i·y_{t−i} + Σ b_i·x_{t−i} + e_t`.
//! - `X` Granger-causes `Y` iff the `x`-lag block improves the fit beyond
//!   chance, measured by the F-statistic
//!   `F = ((RSS_r − RSS_u)/p) / (RSS_u/(T − 2p − 1))`
//!   on `(p, T − 2p − 1)` degrees of freedom, where `T = n − p` is the number of
//!   usable rows after lagging. The p-value is the F upper tail.
//!
//! Causality here is **predictive (Granger) causality, not structural**: it says
//! past-X carries incremental linear information about future-Y given past-Y. It
//! is directional — call it twice with `X`/`Y` swapped to compare directions.
//!
//! Fails closed: length mismatch, non-finite input, `lags == 0`, too few rows for
//! the denominator df (`n < 3p + 2`), a rank-deficient design (collinear
//! regressors, e.g. a constant series), or a degenerate unrestricted fit
//! (`RSS_u ≈ 0`, perfect in-sample fit) — never a silent `NaN`.

use calyx_core::{CalyxError, Result};
use serde::{Deserialize, Serialize};

use crate::special_fn::f_upper_tail_p;

/// Default autoregressive lag order for a single-lag Granger test.
pub const DEFAULT_GRANGER_LAGS: usize = 1;

/// Default Anneal-tunable lag set for the Granger sweep. The sweep reports the
/// max-effect lag (mirrors the transfer-entropy lag sweep doctrine).
pub const DEFAULT_GRANGER_LAG_SWEEP: [usize; 4] = [1, 2, 4, 8];

/// Relative floor on the unrestricted residual sum of squares. If `RSS_u` falls
/// below `RSS_TOTAL · ε`, the unrestricted model fits `Y` essentially perfectly
/// in-sample and the F-ratio is numerically undefined (÷0) — fail closed rather
/// than emit a spurious `+∞`.
const MIN_RSS_FRACTION: f64 = 1e-12;

/// Result of a directional linear Granger-causality F-test `X → Y`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct GrangerReport {
    /// F-statistic for the joint significance of the `x`-lag block.
    pub f_statistic: f32,
    /// Upper-tail p-value on `(df_num, df_den)` degrees of freedom.
    pub p_value: f32,
    /// Lag order `p`.
    pub lags: usize,
    /// Numerator df = `p` (number of restricted coefficients).
    pub df_num: usize,
    /// Denominator df = `T − 2p − 1`.
    pub df_den: usize,
    pub rss_restricted: f32,
    pub rss_unrestricted: f32,
    /// Usable rows after lagging, `T = n − p`.
    pub n_used: usize,
}

/// Test whether `x` Granger-causes `y` at the default lag order.
pub fn granger_causality(x: &[f32], y: &[f32]) -> Result<GrangerReport> {
    granger_causality_lags(x, y, DEFAULT_GRANGER_LAGS)
}

/// Test whether `x` Granger-causes `y` using `p = lags` autoregressive lags.
pub fn granger_causality_lags(x: &[f32], y: &[f32], lags: usize) -> Result<GrangerReport> {
    if lags == 0 {
        return Err(CalyxError::assay_insufficient_samples(
            "Granger causality requires lags ≥ 1",
        ));
    }
    if x.len() != y.len() {
        return Err(CalyxError::assay_insufficient_samples(format!(
            "Granger causality requires paired series: x={} y={}",
            x.len(),
            y.len()
        )));
    }
    let n = x.len();
    let p = lags;
    // Usable rows T = n − p; unrestricted params = 2p + 1; need df_den ≥ 1.
    if n < 3 * p + 2 {
        return Err(CalyxError::assay_insufficient_samples(format!(
            "Granger causality with {p} lags requires at least {} samples (3p+2); got {n}",
            3 * p + 2
        )));
    }
    let xd = to_finite_f64("x", x)?;
    let yd = to_finite_f64("y", y)?;

    let t = n - p; // usable rows
    // Response and design rows for t0 = p .. n-1 (0-indexed target index).
    let mut response = Vec::with_capacity(t);
    // Restricted design: [1, y_{t-1}, .., y_{t-p}].
    let mut restricted = Vec::with_capacity(t);
    // Unrestricted design: restricted ++ [x_{t-1}, .., x_{t-p}].
    let mut unrestricted = Vec::with_capacity(t);
    for target in p..n {
        response.push(yd[target]);
        let mut r_row = Vec::with_capacity(1 + p);
        r_row.push(1.0); // intercept
        for lag in 1..=p {
            r_row.push(yd[target - lag]);
        }
        let mut u_row = r_row.clone();
        for lag in 1..=p {
            u_row.push(xd[target - lag]);
        }
        restricted.push(r_row);
        unrestricted.push(u_row);
    }

    let rss_r = ols_rss(&restricted, &response, 1 + p)?;
    let rss_u = ols_rss(&unrestricted, &response, 1 + 2 * p)?;

    // Total variation of the response (about its mean) bounds RSS from above and
    // sets the scale for the perfect-fit floor.
    let mean_y = response.iter().sum::<f64>() / t as f64;
    let tss = response.iter().map(|&v| (v - mean_y).powi(2)).sum::<f64>();
    if rss_u <= tss * MIN_RSS_FRACTION {
        return Err(CalyxError::assay_degenerate_input(
            "Granger causality undefined: unrestricted model fits Y perfectly (RSS_u ≈ 0)",
        ));
    }

    let df_num = p;
    let df_den = t - (2 * p + 1);
    // Adding regressors cannot increase RSS, so RSS_r ≥ RSS_u; clamp tiny negative
    // numerator from float noise. F ≥ 0 by construction.
    let numerator = ((rss_r - rss_u).max(0.0)) / df_num as f64;
    let denominator = rss_u / df_den as f64;
    let f_statistic = numerator / denominator;
    let p_value = f_upper_tail_p(f_statistic, df_num as f64, df_den as f64)?;

    Ok(GrangerReport {
        f_statistic: f_statistic as f32,
        p_value: p_value as f32,
        lags: p,
        df_num,
        df_den,
        rss_restricted: rss_r as f32,
        rss_unrestricted: rss_u as f32,
        n_used: t,
    })
}

/// Sweep the default lag set `[1, 2, 4, 8]` and return the report for the
/// **max-effect lag** — the lag with the strongest evidence (lowest p-value,
/// F-statistic as the tiebreak). Lags whose sample requirement (`n ≥ 3p+2`)
/// exceeds the series length, or that hit a degenerate fit, are skipped; the
/// call fails closed only if *no* lag in the set is admissible (propagating the
/// last failure so the reason is never hidden).
///
/// Note: this reports the most-associated lag, not a multiplicity-corrected
/// test — the p-value is the per-lag F upper tail at the winning lag.
pub fn granger_causality_sweep(x: &[f32], y: &[f32]) -> Result<GrangerReport> {
    granger_causality_sweep_lags(x, y, &DEFAULT_GRANGER_LAG_SWEEP)
}

/// [`granger_causality_sweep`] over an explicit lag set.
pub fn granger_causality_sweep_lags(x: &[f32], y: &[f32], lags: &[usize]) -> Result<GrangerReport> {
    if lags.is_empty() {
        return Err(CalyxError::assay_insufficient_samples(
            "Granger sweep requires a non-empty lag set",
        ));
    }
    let mut best: Option<GrangerReport> = None;
    let mut last_err: Option<CalyxError> = None;
    for &p in lags {
        match granger_causality_lags(x, y, p) {
            Ok(report) => {
                let take = match &best {
                    None => true,
                    Some(b) => {
                        report.p_value < b.p_value
                            || (report.p_value == b.p_value && report.f_statistic > b.f_statistic)
                    }
                };
                if take {
                    best = Some(report);
                }
            }
            Err(e) => last_err = Some(e),
        }
    }
    best.ok_or_else(|| {
        last_err.unwrap_or_else(|| {
            CalyxError::assay_insufficient_samples("Granger sweep: no admissible lag")
        })
    })
}

// ----- OLS numerics ----------------------------------------------------------

/// Residual sum of squares of the ordinary-least-squares fit of `y` on the
/// design rows `m` (each of width `k`), via the normal equations
/// `(MᵀM)β = Mᵀy`. Fails closed if `MᵀM` is singular (rank-deficient design).
fn ols_rss(m: &[Vec<f64>], y: &[f64], k: usize) -> Result<f64> {
    let t = m.len();
    debug_assert!(t == y.len() && m.iter().all(|r| r.len() == k));
    // Gram matrix A = MᵀM (k×k, symmetric PSD) and rhs = Mᵀy.
    let mut a = vec![0.0f64; k * k];
    let mut rhs = vec![0.0f64; k];
    for (row, &yi) in m.iter().zip(y) {
        for c in 0..k {
            rhs[c] += row[c] * yi;
            for d in c..k {
                a[c * k + d] += row[c] * row[d];
            }
        }
    }
    // Mirror the upper triangle into the lower.
    for c in 0..k {
        for d in (c + 1)..k {
            a[d * k + c] = a[c * k + d];
        }
    }
    let beta = solve_spd(&mut a, &mut rhs, k).ok_or_else(|| {
        CalyxError::assay_degenerate_input(
            "Granger causality undefined: design matrix is rank-deficient (collinear/constant regressors)",
        )
    })?;
    // RSS = Σ (y_i − mᵢ·β)².
    let mut rss = 0.0f64;
    for (row, &yi) in m.iter().zip(y) {
        let fit: f64 = row.iter().zip(&beta).map(|(&mij, &bj)| mij * bj).sum();
        rss += (yi - fit).powi(2);
    }
    Ok(rss)
}

/// Solve `A·β = rhs` for a `k×k` symmetric matrix by Gauss–Jordan elimination
/// with partial pivoting (`A` and `rhs` are overwritten). Returns `None` if `A`
/// is singular to working precision.
fn solve_spd(a: &mut [f64], rhs: &mut [f64], k: usize) -> Option<Vec<f64>> {
    // Scale-aware singularity threshold: relative to the largest diagonal entry.
    let scale = (0..k).map(|i| a[i * k + i].abs()).fold(0.0f64, f64::max);
    let eps = 1e-12 * scale.max(1.0);
    for col in 0..k {
        let mut pivot = col;
        let mut best = a[col * k + col].abs();
        for row in (col + 1)..k {
            let v = a[row * k + col].abs();
            if v > best {
                best = v;
                pivot = row;
            }
        }
        if best < eps {
            return None;
        }
        if pivot != col {
            for j in 0..k {
                a.swap(col * k + j, pivot * k + j);
            }
            rhs.swap(col, pivot);
        }
        let inv_p = 1.0 / a[col * k + col];
        for j in 0..k {
            a[col * k + j] *= inv_p;
        }
        rhs[col] *= inv_p;
        for row in 0..k {
            if row == col {
                continue;
            }
            let factor = a[row * k + col];
            if factor == 0.0 {
                continue;
            }
            for j in 0..k {
                a[row * k + j] -= factor * a[col * k + j];
            }
            rhs[row] -= factor * rhs[col];
        }
    }
    Some(rhs.to_vec())
}

fn to_finite_f64(name: &str, values: &[f32]) -> Result<Vec<f64>> {
    let mut out = Vec::with_capacity(values.len());
    for (idx, &v) in values.iter().enumerate() {
        if !v.is_finite() {
            return Err(CalyxError::assay_insufficient_samples(format!(
                "Granger {name}[{idx}] is not finite ({v})"
            )));
        }
        out.push(v as f64);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    // Synthetic series are generated by index-based AR recurrences (y[t] from
    // y[t-1], x[t-1]), for which a range loop is the natural form.
    #![allow(clippy::needless_range_loop)]
    use super::*;

    /// Deterministic splitmix64 → uniform f64 in [0,1); reproducible noise, no RNG.
    fn splitmix(mut x: u64) -> f64 {
        x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = x;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        ((z >> 11) as f64) / ((1_u64 << 53) as f64)
    }

    #[test]
    fn detects_true_directional_causation() {
        // y_t = 0.5·y_{t-1} + 1.5·x_{t-1} + small noise; x is exogenous noise.
        // X drives Y with a one-step lag → X→Y strongly significant, Y↛X not.
        let n = 120;
        let mut x = vec![0.0f32; n];
        let mut y = vec![0.0f32; n];
        for t in 0..n {
            x[t] = (splitmix(t as u64) - 0.5) as f32;
        }
        for t in 1..n {
            let noise = (splitmix(1000 + t as u64) - 0.5) * 0.2;
            y[t] = 0.5 * y[t - 1] + 1.5 * x[t - 1] + noise as f32;
        }
        let fwd = granger_causality(&x, &y).unwrap();
        let rev = granger_causality(&y, &x).unwrap();
        assert!(fwd.p_value < 0.001, "X→Y must be significant: {fwd:?}");
        assert!(rev.p_value > 0.05, "Y→X must be insignificant: {rev:?}");
        assert!(
            fwd.rss_unrestricted < fwd.rss_restricted,
            "x-lags improve fit: {fwd:?}"
        );
    }

    #[test]
    fn independent_series_is_insignificant() {
        let n = 100;
        let mut x = vec![0.0f32; n];
        let mut y = vec![0.0f32; n];
        for t in 0..n {
            x[t] = (splitmix(t as u64) - 0.5) as f32;
            y[t] = (splitmix(7777 + t as u64) - 0.5) as f32;
        }
        let g = granger_causality(&x, &y).unwrap();
        assert!(g.p_value > 0.05, "independent → insignificant: {g:?}");
    }

    #[test]
    fn f_statistic_matches_independent_ols() {
        // Locked against an independent numpy `lstsq` OLS on the SAME lagged
        // design: for this fixed 12-point series at p=1,
        //   T = 11, RSS_r = 96.909059, RSS_u = 1.330807, df = (1, 8),
        //   F = 574.5581, upper-tail p = 9.78e-9.
        let x = [
            1.0f32, 3.0, 2.0, 5.0, 4.0, 7.0, 6.0, 9.0, 8.0, 11.0, 10.0, 13.0,
        ];
        let y = [
            2.0f32, 1.0, 5.0, 4.0, 8.0, 6.0, 11.0, 9.0, 14.0, 12.0, 17.0, 15.0,
        ];
        let g = granger_causality_lags(&x, &y, 1).unwrap();
        assert_eq!(g.df_num, 1);
        assert_eq!(g.df_den, 8);
        assert_eq!(g.n_used, 11);
        assert!((g.rss_restricted - 96.909_06).abs() < 1e-2, "RSS_r: {g:?}");
        assert!(
            (g.rss_unrestricted - 1.330_807).abs() < 1e-3,
            "RSS_u: {g:?}"
        );
        // Large F; assert to a relative tolerance and a decisive p.
        assert!((g.f_statistic - 574.558).abs() / 574.558 < 1e-3, "F: {g:?}");
        assert!(g.p_value < 1e-6, "p decisive: {g:?}");
    }

    #[test]
    fn sweep_selects_the_planted_lag() {
        // Plant the driver at a two-step lag: y_t = 0.4·y_{t-1} + 1.5·x_{t-2}.
        // The sweep over [1,2,4,8] should pick lag 2 as the max-effect lag.
        let n = 160;
        let mut x = vec![0.0f32; n];
        let mut y = vec![0.0f32; n];
        for t in 0..n {
            x[t] = (splitmix(t as u64) - 0.5) as f32;
        }
        for t in 2..n {
            let noise = (splitmix(2000 + t as u64) - 0.5) * 0.15;
            y[t] = 0.4 * y[t - 1] + 1.5 * x[t - 2] + noise as f32;
        }
        let best = granger_causality_sweep(&x, &y).unwrap();
        assert_eq!(
            best.lags, 2,
            "sweep must find the planted lag-2 driver: {best:?}"
        );
        assert!(best.p_value < 0.001, "planted lag significant: {best:?}");
    }

    #[test]
    fn fails_closed_on_zero_lags() {
        assert_eq!(
            granger_causality_lags(&[1.0, 2.0, 3.0, 4.0, 5.0], &[1.0, 2.0, 3.0, 4.0, 5.0], 0)
                .unwrap_err()
                .code,
            "CALYX_ASSAY_INSUFFICIENT_SAMPLES"
        );
    }

    #[test]
    fn fails_closed_on_length_mismatch() {
        assert_eq!(
            granger_causality(&[1.0, 2.0, 3.0, 4.0, 5.0], &[1.0, 2.0, 3.0])
                .unwrap_err()
                .code,
            "CALYX_ASSAY_INSUFFICIENT_SAMPLES"
        );
    }

    #[test]
    fn fails_closed_below_min_samples() {
        // p=1 needs n ≥ 5; give 4.
        assert_eq!(
            granger_causality_lags(&[1.0, 2.0, 3.0, 4.0], &[2.0, 1.0, 4.0, 3.0], 1)
                .unwrap_err()
                .code,
            "CALYX_ASSAY_INSUFFICIENT_SAMPLES"
        );
    }

    #[test]
    fn fails_closed_on_non_finite() {
        let mut x = vec![1.0f32; 20];
        x[5] = f32::NAN;
        let y: Vec<f32> = (0..20).map(|i| i as f32).collect();
        assert_eq!(
            granger_causality(&x, &y).unwrap_err().code,
            "CALYX_ASSAY_INSUFFICIENT_SAMPLES"
        );
    }

    #[test]
    fn fails_closed_on_constant_regressor() {
        // Constant y → restricted design has collinear [1, y_{t-1}] columns →
        // rank-deficient normal equations → degenerate.
        let x: Vec<f32> = (0..20).map(|i| (i % 3) as f32).collect();
        let y = vec![4.0f32; 20];
        assert_eq!(
            granger_causality(&x, &y).unwrap_err().code,
            "CALYX_ASSAY_DEGENERATE_INPUT"
        );
    }
}
