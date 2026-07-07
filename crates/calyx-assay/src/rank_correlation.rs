//! Rank correlation: Spearman's ρ and Kendall's τ-b — robust monotone
//! association measures for the heavy-tailed, tie-prone market series that
//! linear (Pearson) correlation mishandles (#57).
//!
//! Both estimators are tie-correct by construction:
//! - **Spearman ρ** is the Pearson correlation of *mid-ranks* (average ranks
//!   within tie groups). The textbook `1 − 6Σd²/(n(n²−1))` shortcut is biased
//!   under ties and is deliberately not used. Significance uses the exact
//!   Student-t statistic `t = ρ·√((n−2)/(1−ρ²))` on `df = n−2`; the confidence
//!   interval uses the Fisher z-transform with the Bonett–Wright (2000)
//!   standard error, which is calibrated for ranks (Pearson's `1/√(n−3)` is
//!   anticonservative for ρ).
//! - **Kendall τ-b** uses the tie-adjusted denominator
//!   `√((n₀−n₁)(n₀−n₂))` and the Hollander–Wolfe (1999) tie-corrected variance
//!   for the asymptotic normal p-value (the same construction `scipy.stats`
//!   uses when ties are present).
//!
//! Everything fails closed: length mismatch, non-finite values, `n < 3`, or a
//! constant (zero-variance / all-tied) column returns a structured
//! `CalyxError`, never a silent `NaN`.

use calyx_core::{CalyxError, Result};
use serde::{Deserialize, Serialize};

use crate::special_fn::{normal_two_sided_p, student_t_two_sided_p};

/// Minimum paired observations for a defined rank-correlation test. Below 3 the
/// Student-t `df = n−2` and the Kendall `n(n−1)(n−2)` variance term vanish.
pub const MIN_RANK_CORR_SAMPLES: usize = 3;

/// Two-sided standard-normal 95% quantile, for the Fisher-z confidence interval.
const Z_95: f64 = 1.959_963_984_540_054;

/// Spearman rank-correlation result with exact-t significance and a
/// Fisher-z (Bonett–Wright) 95% confidence interval.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct SpearmanReport {
    pub rho: f32,
    pub t_statistic: f32,
    pub p_value: f32,
    pub ci_low: f32,
    pub ci_high: f32,
    pub n_samples: usize,
}

/// Kendall τ-b result with the raw sign statistic `S = C − D`, the tie-corrected
/// asymptotic z, and the two-sided normal p-value.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct KendallReport {
    pub tau_b: f32,
    pub s_statistic: i64,
    pub z_statistic: f32,
    pub p_value: f32,
    pub n_concordant: u64,
    pub n_discordant: u64,
    pub n_samples: usize,
}

/// Spearman's ρ over paired samples (Pearson-on-mid-ranks, tie-correct).
pub fn spearman_rho(x: &[f32], y: &[f32]) -> Result<SpearmanReport> {
    let (xd, yd) = validate_pair("Spearman ρ", x, y)?;
    let n = xd.len();

    let rx = midranks(&xd);
    let ry = midranks(&yd);
    let rho = pearson(&rx, &ry).ok_or_else(|| {
        CalyxError::assay_degenerate_input(
            "Spearman ρ undefined: a column is constant (all ranks tied)",
        )
    })?;
    let rho = rho.clamp(-1.0, 1.0);

    let df = (n - 2) as f64;
    let one_minus = 1.0 - rho * rho;
    let (t_statistic, p_value) = if one_minus <= f64::EPSILON {
        // Perfect monotone relationship: t → ∞, maximal significance.
        (rho.signum() * f64::INFINITY, 0.0)
    } else {
        let t = rho * (df / one_minus).sqrt();
        (t, student_t_two_sided_p(t, df)?)
    };

    // Fisher z-transform CI with the Bonett–Wright rank standard error.
    let z = atanh(rho);
    let se = ((1.0 + rho * rho / 2.0) / (n as f64 - 3.0)).sqrt();
    let (ci_low, ci_high) = if (n as f64 - 3.0) > 0.0 && se.is_finite() {
        (tanh(z - Z_95 * se), tanh(z + Z_95 * se))
    } else {
        // n = 3: the Fisher SE is undefined; report the point estimate.
        (rho, rho)
    };

    Ok(SpearmanReport {
        rho: rho as f32,
        t_statistic: t_statistic as f32,
        p_value: p_value as f32,
        ci_low: ci_low.clamp(-1.0, 1.0) as f32,
        ci_high: ci_high.clamp(-1.0, 1.0) as f32,
        n_samples: n,
    })
}

/// Kendall's τ-b over paired samples (tie-adjusted, Hollander–Wolfe variance).
pub fn kendall_tau_b(x: &[f32], y: &[f32]) -> Result<KendallReport> {
    let (xd, yd) = validate_pair("Kendall τ-b", x, y)?;
    let n = xd.len();

    // Pairwise concordance in O(n²) — exact, and n is bounded by the analysis
    // window in the association fan-out, not the whole corpus.
    let mut concordant: u64 = 0;
    let mut discordant: u64 = 0;
    for i in 0..n {
        for j in (i + 1)..n {
            let dx = (xd[j] - xd[i]).partial_cmp(&0.0).unwrap() as i32;
            let dy = (yd[j] - yd[i]).partial_cmp(&0.0).unwrap() as i32;
            let sign = dx.signum() * dy.signum();
            if sign > 0 {
                concordant += 1;
            } else if sign < 0 {
                discordant += 1;
            }
        }
    }
    let s = concordant as i64 - discordant as i64;

    let nf = n as f64;
    let n0 = nf * (nf - 1.0) / 2.0;
    let tx = tie_group_sizes(&xd);
    let ty = tie_group_sizes(&yd);
    let n1: f64 = tx.iter().map(|&t| tie_pairs(t)).sum();
    let n2: f64 = ty.iter().map(|&t| tie_pairs(t)).sum();

    // n1,n2 ≤ n0 (tie pairs ≤ total pairs) ⇒ the product is ≥ 0 and `denom` is
    // a finite non-negative real; it is exactly 0 only when a column is fully
    // tied, which makes τ-b undefined.
    let denom = ((n0 - n1) * (n0 - n2)).sqrt();
    if denom <= 0.0 {
        return Err(CalyxError::assay_degenerate_input(
            "Kendall τ-b undefined: a column is constant (no untied pairs)",
        ));
    }
    let tau_b = (s as f64 / denom).clamp(-1.0, 1.0);

    // Hollander–Wolfe (1999) tie-corrected variance of S under H₀.
    let v0 = nf * (nf - 1.0) * (2.0 * nf + 5.0);
    let vt: f64 = tx.iter().map(|&t| tie_var_term(t)).sum();
    let vu: f64 = ty.iter().map(|&t| tie_var_term(t)).sum();
    let v1_num: f64 = tx.iter().map(|&t| tie_pairs(t)).sum::<f64>()
        * ty.iter().map(|&t| tie_pairs(t)).sum::<f64>();
    let v2_num: f64 = tx.iter().map(|&t| tie_triples(t)).sum::<f64>()
        * ty.iter().map(|&t| tie_triples(t)).sum::<f64>();
    let variance = (v0 - vt - vu) / 18.0
        + v1_num / (2.0 * nf * (nf - 1.0))
        + v2_num / (9.0 * nf * (nf - 1.0) * (nf - 2.0));

    let (z_statistic, p_value) = if variance > 0.0 {
        let z = s as f64 / variance.sqrt();
        (z, normal_two_sided_p(z)?)
    } else {
        // S must be 0 when variance is 0 (fully tied structure with defined
        // denom cannot happen here, but stay fail-safe): no evidence.
        (0.0, 1.0)
    };

    Ok(KendallReport {
        tau_b: tau_b as f32,
        s_statistic: s,
        z_statistic: z_statistic as f32,
        p_value: p_value as f32,
        n_concordant: concordant,
        n_discordant: discordant,
        n_samples: n,
    })
}

// ----- shared numerics -------------------------------------------------------

fn validate_pair(what: &str, x: &[f32], y: &[f32]) -> Result<(Vec<f64>, Vec<f64>)> {
    if x.len() != y.len() {
        return Err(CalyxError::assay_insufficient_samples(format!(
            "{what} requires paired samples: x={} y={}",
            x.len(),
            y.len()
        )));
    }
    if x.len() < MIN_RANK_CORR_SAMPLES {
        return Err(CalyxError::assay_insufficient_samples(format!(
            "{what} requires at least {MIN_RANK_CORR_SAMPLES} paired samples; got {}",
            x.len()
        )));
    }
    let xd = to_finite_f64(what, "x", x)?;
    let yd = to_finite_f64(what, "y", y)?;
    Ok((xd, yd))
}

fn to_finite_f64(what: &str, name: &str, values: &[f32]) -> Result<Vec<f64>> {
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

/// Average ranks (1-based), ties resolved to the mean of their rank block.
fn midranks(values: &[f64]) -> Vec<f64> {
    let n = values.len();
    let mut idx: Vec<usize> = (0..n).collect();
    idx.sort_by(|&a, &b| values[a].partial_cmp(&values[b]).expect("finite-validated"));
    let mut ranks = vec![0.0f64; n];
    let mut i = 0;
    while i < n {
        let mut j = i + 1;
        while j < n && values[idx[j]] == values[idx[i]] {
            j += 1;
        }
        // 0-based positions i..j map to 1-based ranks (i+1)..=j; their mean:
        let avg = ((i + 1 + j) as f64) / 2.0;
        for &orig in &idx[i..j] {
            ranks[orig] = avg;
        }
        i = j;
    }
    ranks
}

/// Sizes of the equal-value groups (used for the tie corrections).
fn tie_group_sizes(values: &[f64]) -> Vec<usize> {
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).expect("finite-validated"));
    let mut groups = Vec::new();
    let mut i = 0;
    while i < sorted.len() {
        let mut j = i + 1;
        while j < sorted.len() && sorted[j] == sorted[i] {
            j += 1;
        }
        groups.push(j - i);
        i = j;
    }
    groups
}

fn tie_pairs(t: usize) -> f64 {
    let t = t as f64;
    t * (t - 1.0) / 2.0
}

fn tie_triples(t: usize) -> f64 {
    let t = t as f64;
    t * (t - 1.0) * (t - 2.0)
}

fn tie_var_term(t: usize) -> f64 {
    let t = t as f64;
    t * (t - 1.0) * (2.0 * t + 5.0)
}

/// Pearson correlation of two equal-length vectors; `None` if either is
/// constant (zero variance → correlation undefined).
fn pearson(x: &[f64], y: &[f64]) -> Option<f64> {
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
    Some(cov / (vx.sqrt() * vy.sqrt()))
}

fn atanh(r: f64) -> f64 {
    0.5 * ((1.0 + r) / (1.0 - r)).ln()
}

fn tanh(z: f64) -> f64 {
    z.tanh()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(actual: f32, expected: f32, tol: f32, what: &str) {
        assert!(
            (actual - expected).abs() <= tol,
            "{what}: got {actual}, expected {expected} (tol {tol})"
        );
    }

    #[test]
    fn spearman_perfect_monotone_is_one() {
        let x = [1.0f32, 2.0, 3.0, 4.0, 5.0];
        let y = [10.0f32, 20.0, 30.0, 40.0, 50.0]; // strictly increasing, non-linear ok
        let r = spearman_rho(&x, &y).unwrap();
        approx(r.rho, 1.0, 1e-6, "ρ");
        assert!(r.p_value < 1e-6, "perfect ρ must be significant: {r:?}");
        approx(r.ci_high, 1.0, 1e-6, "ci_high");
    }

    #[test]
    fn spearman_perfect_antitone_is_minus_one() {
        let x = [1.0f32, 2.0, 3.0, 4.0, 5.0];
        let y = [5.0f32, 4.0, 3.0, 2.0, 1.0];
        let r = spearman_rho(&x, &y).unwrap();
        approx(r.rho, -1.0, 1e-6, "ρ");
    }

    #[test]
    fn spearman_matches_known_tie_corrected_value() {
        // x has a tie (two 2.0). Reference (R cor(method="spearman")): ρ = 0.9746794.
        let x = [1.0f32, 2.0, 2.0, 4.0, 5.0];
        let y = [1.0f32, 2.0, 3.0, 4.0, 5.0];
        let r = spearman_rho(&x, &y).unwrap();
        approx(r.rho, 0.974_679_4, 1e-5, "tie-corrected ρ");
    }

    #[test]
    fn kendall_perfect_monotone_tau_is_one() {
        let x = [1.0f32, 2.0, 3.0, 4.0, 5.0];
        let y = [2.0f32, 4.0, 6.0, 8.0, 10.0];
        let k = kendall_tau_b(&x, &y).unwrap();
        approx(k.tau_b, 1.0, 1e-6, "τ-b");
        assert_eq!(k.n_discordant, 0);
        assert_eq!(k.n_concordant, 10); // C(5,2)
    }

    #[test]
    fn kendall_matches_known_tie_corrected_value() {
        // Hand-verified: x=[1,2,2,4,5], y=[1,2,3,4,5]. Of the 10 pairs, the
        // (2,2) x-tie leaves 9 concordant, 0 discordant, S=9. n0=10, n1=1
        // (one x tie-pair), n2=0 → τ-b = 9/√(9·10) = 0.9486833.
        // Variance (Hollander–Wolfe) = (300−18)/18 = 15.6̄ → z = 9/√15.6̄ =
        // 2.27379 → two-sided normal p ≈ 0.02298.
        let x = [1.0f32, 2.0, 2.0, 4.0, 5.0];
        let y = [1.0f32, 2.0, 3.0, 4.0, 5.0];
        let k = kendall_tau_b(&x, &y).unwrap();
        assert_eq!(k.n_concordant, 9);
        assert_eq!(k.n_discordant, 0);
        assert_eq!(k.s_statistic, 9);
        approx(k.tau_b, 0.948_683_3, 1e-5, "tie-corrected τ-b");
        approx(k.z_statistic, 2.273_79, 1e-3, "τ-b z");
        approx(k.p_value, 0.022_98, 5e-4, "τ-b p-value");
    }

    #[test]
    fn independent_series_is_near_zero_and_insignificant() {
        // Interleaved with no monotone trend.
        let x = [1.0f32, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0];
        let y = [5.0f32, 3.0, 6.0, 2.0, 7.0, 4.0, 8.0, 1.0];
        let s = spearman_rho(&x, &y).unwrap();
        let k = kendall_tau_b(&x, &y).unwrap();
        assert!(s.rho.abs() < 0.6, "ρ should be weak: {s:?}");
        assert!(s.p_value > 0.1, "weak ρ should be insignificant: {s:?}");
        assert!(k.tau_b.abs() < 0.6, "τ should be weak: {k:?}");
    }

    #[test]
    fn fails_closed_on_length_mismatch() {
        let err = spearman_rho(&[1.0, 2.0, 3.0], &[1.0, 2.0]).unwrap_err();
        assert_eq!(err.code, "CALYX_ASSAY_INSUFFICIENT_SAMPLES");
    }

    #[test]
    fn fails_closed_below_min_samples() {
        let err = kendall_tau_b(&[1.0, 2.0], &[1.0, 2.0]).unwrap_err();
        assert_eq!(err.code, "CALYX_ASSAY_INSUFFICIENT_SAMPLES");
    }

    #[test]
    fn fails_closed_on_non_finite() {
        let err = spearman_rho(&[1.0, f32::NAN, 3.0], &[1.0, 2.0, 3.0]).unwrap_err();
        assert_eq!(err.code, "CALYX_ASSAY_INSUFFICIENT_SAMPLES");
    }

    #[test]
    fn fails_closed_on_constant_column() {
        let s = spearman_rho(&[2.0, 2.0, 2.0, 2.0], &[1.0, 2.0, 3.0, 4.0]).unwrap_err();
        assert_eq!(s.code, "CALYX_ASSAY_DEGENERATE_INPUT");
        let k = kendall_tau_b(&[2.0, 2.0, 2.0, 2.0], &[1.0, 2.0, 3.0, 4.0]).unwrap_err();
        assert_eq!(k.code, "CALYX_ASSAY_DEGENERATE_INPUT");
    }
}
