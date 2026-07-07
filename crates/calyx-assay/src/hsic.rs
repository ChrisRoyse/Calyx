//! HSIC — the Hilbert–Schmidt Independence Criterion, a kernel independence test
//! that is **0 iff X and Y are independent** in the RKHS sense (#55). Unlike the
//! k-NN mutual-information estimators (KSG), whose bias and variance degrade in
//! high dimension, HSIC stays stable — it is the market screen's robust,
//! non-parametric independence test, complementing the linear/rank/dCor family.
//!
//! Construction (Gretton, Bousquet, Smola, Schölkopf 2005; Gretton et al. 2008):
//! - Gaussian RBF Gram matrices `K_ij = exp(−‖x_i−x_j‖²/(2σ²))`, `L` likewise;
//!   `σ` defaults to the **median-pairwise-distance heuristic**
//!   `σ = √(median{‖x_i−x_j‖² : i<j}/2)` per variable.
//! - **Biased** estimator `HSIC_b = n⁻²·tr(K_c L_c)` where `K_c = HKH` is the
//!   double-centred Gram matrix (`H = I − 11ᵀ/n`).
//! - **Unbiased** estimator (Song et al. 2012, `n ≥ 4`) from the diagonal-zeroed
//!   `K̃, L̃`:
//!   `HSIC_u = [tr(K̃L̃) + (1ᵀK̃1)(1ᵀL̃1)/((n−1)(n−2)) − 2/(n−2)·1ᵀK̃L̃1] / (n(n−3))`.
//!
//! Significance has two paths:
//! - **Closed-form gamma approximation** (Gretton 2008): under H₀ the statistic
//!   `T = n·HSIC_b` is Gamma-distributed with moments matched to the null;
//!   `p = 1 − F_Γ(T; α, β)` with `α = mean²/var`, `β = var·n/mean`. This is the
//!   "stable, no-permutation" test the issue asks for; it is asymptotic and needs
//!   a reasonable `n` (variance is only defined for `n ≥ 6`).
//! - **Seeded permutation test** ([`hsic_permutation_test`]) — exact-valid at any
//!   `n ≥ 4` via the add-one estimator `(1+#ge)/(1+P)`, for small-sample rigor.
//!
//! Fails closed on length mismatch, non-finite input, too few samples, or a
//! constant (zero median distance ⇒ undefined bandwidth) column — never `NaN`.

use calyx_core::{CalyxError, Result};
use rand::SeedableRng;
use rand::seq::SliceRandom;
use rand_chacha::ChaCha8Rng;
use serde::{Deserialize, Serialize};

use crate::special_fn::gammq;

/// Minimum samples for the biased/unbiased HSIC point estimates (unbiased needs
/// `n(n−3)` and `(n−1)(n−2)` denominators, i.e. `n ≥ 4`).
pub const MIN_HSIC_SAMPLES: usize = 4;
/// Minimum samples for the closed-form gamma test (null variance carries the
/// factor `(n−4)(n−5)`, so it is positive only for `n ≥ 6`).
pub const MIN_HSIC_GAMMA_SAMPLES: usize = 6;
/// Default permutation count for the permutation independence test.
pub const DEFAULT_HSIC_PERMUTATIONS: usize = 999;
/// Default deterministic seed for the permutation null.
pub const DEFAULT_HSIC_SEED: u64 = 0x0451_C0DE_0DE5_EED5;

/// Kernel-bandwidth configuration. `None` selects the median-distance heuristic
/// for that variable; `Some(σ)` pins a fixed bandwidth (e.g. for reproducible
/// regression checks).
#[derive(Clone, Copy, Debug, Default)]
pub struct HsicConfig {
    pub bandwidth_x: Option<f64>,
    pub bandwidth_y: Option<f64>,
}

/// Configuration for the permutation independence test.
#[derive(Clone, Copy, Debug)]
pub struct HsicPermConfig {
    pub kernel: HsicConfig,
    pub permutations: usize,
    pub seed: u64,
}

impl Default for HsicPermConfig {
    fn default() -> Self {
        Self {
            kernel: HsicConfig::default(),
            permutations: DEFAULT_HSIC_PERMUTATIONS,
            seed: DEFAULT_HSIC_SEED,
        }
    }
}

/// HSIC point estimates (no p-value).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct HsicEstimators {
    pub hsic_biased: f32,
    pub hsic_unbiased: f32,
    pub bandwidth_x: f32,
    pub bandwidth_y: f32,
    pub n_samples: usize,
}

/// HSIC with the closed-form gamma-approximation independence p-value.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct HsicReport {
    pub hsic_biased: f32,
    pub hsic_unbiased: f32,
    /// Test statistic `T = n·HSIC_b` fed to the gamma null.
    pub test_statistic: f32,
    pub p_value: f32,
    pub gamma_shape: f32,
    pub gamma_scale: f32,
    pub bandwidth_x: f32,
    pub bandwidth_y: f32,
    pub n_samples: usize,
}

/// HSIC with a seeded permutation independence p-value.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct HsicTest {
    pub hsic_biased: f32,
    pub p_value: f32,
    pub permutations: usize,
    pub ge_count: usize,
    pub seed: u64,
    pub n_samples: usize,
}

/// HSIC biased/unbiased point estimates with the default (median-heuristic)
/// kernel.
pub fn hsic_estimators(x: &[f32], y: &[f32]) -> Result<HsicEstimators> {
    hsic_estimators_with_config(x, y, HsicConfig::default())
}

/// HSIC point estimates with an explicit kernel configuration.
pub fn hsic_estimators_with_config(
    x: &[f32],
    y: &[f32],
    config: HsicConfig,
) -> Result<HsicEstimators> {
    let core = HsicCore::build(x, y, config)?;
    Ok(HsicEstimators {
        hsic_biased: core.hsic_biased as f32,
        hsic_unbiased: core.hsic_unbiased as f32,
        bandwidth_x: core.sigma_x as f32,
        bandwidth_y: core.sigma_y as f32,
        n_samples: core.n,
    })
}

/// HSIC with the closed-form gamma-approximation p-value (median-heuristic kernel).
pub fn hsic(x: &[f32], y: &[f32]) -> Result<HsicReport> {
    hsic_with_config(x, y, HsicConfig::default())
}

/// HSIC gamma test with an explicit kernel configuration.
pub fn hsic_with_config(x: &[f32], y: &[f32], config: HsicConfig) -> Result<HsicReport> {
    let core = HsicCore::build(x, y, config)?;
    let n = core.n;
    if n < MIN_HSIC_GAMMA_SAMPLES {
        return Err(CalyxError::assay_insufficient_samples(format!(
            "HSIC gamma test requires at least {MIN_HSIC_GAMMA_SAMPLES} samples (null variance needs n≥6); got {n}. Use hsic_permutation_test for small n"
        )));
    }
    let nf = n as f64;

    // Statistic T = n·HSIC_b = tr(K_c L_c)/n.
    let test_statistic = core.tr_kc_lc / nf;

    // Null mean from the diagonal-zeroed RAW Gram sums.
    let mu_x = core.off_diag_sum_k / (nf * (nf - 1.0));
    let mu_y = core.off_diag_sum_l / (nf * (nf - 1.0));
    let mean = (1.0 + mu_x * mu_y - mu_x - mu_y) / nf;

    // Null variance: 2(n-4)(n-5)/(n(n-1)(n-2)(n-3)) · 1/(n(n-1)) · Σ_{i≠j} (Kc_ij Lc_ij)².
    let var_prefactor = 2.0 * (nf - 4.0) * (nf - 5.0)
        / (nf * (nf - 1.0) * (nf - 2.0) * (nf - 3.0))
        / (nf * (nf - 1.0));
    let var = var_prefactor * core.sum_sq_centered_offdiag;

    if mean.is_nan() || mean <= 0.0 || var.is_nan() || var <= 0.0 {
        return Err(CalyxError::assay_degenerate_input(
            "HSIC gamma test undefined: non-positive null moment (degenerate kernel structure)",
        ));
    }
    // Gamma(shape α, scale β): mean α·β = n·mean = E[T]; β carries the extra ·n.
    let shape = mean * mean / var;
    let scale = var * nf / mean;
    // p = 1 − F_Γ(T; α, β) = Q(α, T/β) (regularised upper incomplete gamma).
    let p_value = gammq(shape, test_statistic / scale)?;

    Ok(HsicReport {
        hsic_biased: core.hsic_biased as f32,
        hsic_unbiased: core.hsic_unbiased as f32,
        test_statistic: test_statistic as f32,
        p_value: p_value as f32,
        gamma_shape: shape as f32,
        gamma_scale: scale as f32,
        bandwidth_x: core.sigma_x as f32,
        bandwidth_y: core.sigma_y as f32,
        n_samples: n,
    })
}

/// HSIC independence test via a seeded permutation null — exact-valid at any
/// `n ≥ 4`. The null shuffles Y's sample order (equivalently re-indexes the
/// centred `L_c`, since centering commutes with permutation) and counts how often
/// the permuted `HSIC_b` reaches the observed value; the p-value is the add-one
/// estimator `(1 + #ge)/(1 + P)`.
pub fn hsic_permutation_test(x: &[f32], y: &[f32], config: HsicPermConfig) -> Result<HsicTest> {
    if config.permutations == 0 {
        return Err(CalyxError::assay_insufficient_samples(
            "HSIC permutation test requires permutations > 0",
        ));
    }
    let core = HsicCore::build(x, y, config.kernel)?;
    let n = core.n;
    let observed = core.tr_kc_lc; // ∝ HSIC_b; permutation-invariant denominators
    let tol = 1e-12 * observed.abs().max(1.0);
    let mut rng = ChaCha8Rng::seed_from_u64(config.seed);
    let mut perm: Vec<usize> = (0..n).collect();
    let mut ge_count = 0usize;
    for _ in 0..config.permutations {
        perm.shuffle(&mut rng);
        let mut acc = 0.0f64;
        for i in 0..n {
            let ci = i * n;
            let pi = perm[i] * n;
            for (j, &pj) in perm.iter().enumerate() {
                acc += core.kc[ci + j] * core.lc[pi + pj];
            }
        }
        if acc >= observed - tol {
            ge_count += 1;
        }
    }
    let p_value = (1.0 + ge_count as f64) / (1.0 + config.permutations as f64);
    Ok(HsicTest {
        hsic_biased: core.hsic_biased as f32,
        p_value: p_value as f32,
        permutations: config.permutations,
        ge_count,
        seed: config.seed,
        n_samples: n,
    })
}

// ----- shared core -----------------------------------------------------------

/// Precomputed HSIC quantities shared by every entry point.
struct HsicCore {
    n: usize,
    sigma_x: f64,
    sigma_y: f64,
    hsic_biased: f64,
    hsic_unbiased: f64,
    /// Centred Gram matrices `K_c = HKH`, `L_c = HLH` (row-major n×n).
    kc: Vec<f64>,
    lc: Vec<f64>,
    /// `tr(K_c L_c) = Σ_ij Kc_ij Lc_ij` (so HSIC_b = this / n²).
    tr_kc_lc: f64,
    /// `Σ_{i≠j} K_ij` and `Σ_{i≠j} L_ij` on the RAW Gram matrices.
    off_diag_sum_k: f64,
    off_diag_sum_l: f64,
    /// `Σ_{i≠j} (Kc_ij Lc_ij)²`.
    sum_sq_centered_offdiag: f64,
}

impl HsicCore {
    fn build(x: &[f32], y: &[f32], config: HsicConfig) -> Result<Self> {
        if x.len() != y.len() {
            return Err(CalyxError::assay_insufficient_samples(format!(
                "HSIC requires paired samples: x={} y={}",
                x.len(),
                y.len()
            )));
        }
        let n = x.len();
        if n < MIN_HSIC_SAMPLES {
            return Err(CalyxError::assay_insufficient_samples(format!(
                "HSIC requires at least {MIN_HSIC_SAMPLES} paired samples; got {n}"
            )));
        }
        let xd = to_finite_f64("x", x)?;
        let yd = to_finite_f64("y", y)?;
        let sigma_x = resolve_bandwidth("x", &xd, config.bandwidth_x)?;
        let sigma_y = resolve_bandwidth("y", &yd, config.bandwidth_y)?;

        let k = gaussian_gram(&xd, sigma_x);
        let l = gaussian_gram(&yd, sigma_y);
        let off_diag_sum_k = off_diagonal_sum(&k, n);
        let off_diag_sum_l = off_diagonal_sum(&l, n);

        let kc = double_center(&k, n);
        let lc = double_center(&l, n);

        // tr(K_c L_c) and Σ_{i≠j}(Kc·Lc)².
        let mut tr_kc_lc = 0.0f64;
        let mut sum_sq_centered_offdiag = 0.0f64;
        for i in 0..n {
            for j in 0..n {
                let prod = kc[i * n + j] * lc[i * n + j];
                tr_kc_lc += prod;
                if i != j {
                    sum_sq_centered_offdiag += prod * prod;
                }
            }
        }
        let nf = n as f64;
        let hsic_biased = (tr_kc_lc / (nf * nf)).max(0.0);

        // Unbiased estimator from diagonal-zeroed raw Grams.
        let hsic_unbiased = unbiased_hsic(&k, &l, n);

        Ok(Self {
            n,
            sigma_x,
            sigma_y,
            hsic_biased,
            hsic_unbiased,
            kc,
            lc,
            tr_kc_lc,
            off_diag_sum_k,
            off_diag_sum_l,
            sum_sq_centered_offdiag,
        })
    }
}

/// Unbiased HSIC (Song et al. 2012) from raw Gram matrices; diagonals treated as
/// zero. `n ≥ 4` is guaranteed by the caller.
fn unbiased_hsic(k: &[f64], l: &[f64], n: usize) -> f64 {
    let nf = n as f64;
    let mut tr = 0.0f64; // Σ_{i≠j} K̃_ij L̃_ij
    let mut sum_k = 0.0f64; // 1ᵀK̃1
    let mut sum_l = 0.0f64; // 1ᵀL̃1
    // Row sums of K̃ and L̃ for 1ᵀK̃L̃1 = Σ_k rowK̃_k · rowL̃_k.
    let mut row_k = vec![0.0f64; n];
    let mut row_l = vec![0.0f64; n];
    for i in 0..n {
        for j in 0..n {
            if i == j {
                continue;
            }
            let kij = k[i * n + j];
            let lij = l[i * n + j];
            tr += kij * lij;
            sum_k += kij;
            sum_l += lij;
            row_k[i] += kij;
            row_l[i] += lij;
        }
    }
    let one_kl_one: f64 = (0..n).map(|i| row_k[i] * row_l[i]).sum();
    (tr + sum_k * sum_l / ((nf - 1.0) * (nf - 2.0)) - 2.0 / (nf - 2.0) * one_kl_one)
        / (nf * (nf - 3.0))
}

/// Gaussian RBF Gram matrix (row-major n×n) with bandwidth `sigma`.
fn gaussian_gram(v: &[f64], sigma: f64) -> Vec<f64> {
    let n = v.len();
    let denom = 2.0 * sigma * sigma;
    let mut g = vec![0.0f64; n * n];
    for i in 0..n {
        g[i * n + i] = 1.0;
        for j in (i + 1)..n {
            let d = v[i] - v[j];
            let val = (-(d * d) / denom).exp();
            g[i * n + j] = val;
            g[j * n + i] = val;
        }
    }
    g
}

/// Row-major double-centred matrix `K_c = HKH` via row/col/grand means (O(n²)).
fn double_center(k: &[f64], n: usize) -> Vec<f64> {
    let nf = n as f64;
    let mut row = vec![0.0f64; n];
    for (i, r) in row.iter_mut().enumerate() {
        let mut s = 0.0;
        for j in 0..n {
            s += k[i * n + j];
        }
        *r = s / nf;
    }
    let grand = row.iter().sum::<f64>() / nf;
    let mut kc = vec![0.0f64; n * n];
    for i in 0..n {
        for j in 0..n {
            // K symmetric ⇒ column mean_j == row mean_j.
            kc[i * n + j] = k[i * n + j] - row[i] - row[j] + grand;
        }
    }
    kc
}

fn off_diagonal_sum(g: &[f64], n: usize) -> f64 {
    let mut s = 0.0f64;
    for i in 0..n {
        for j in 0..n {
            if i != j {
                s += g[i * n + j];
            }
        }
    }
    s
}

/// Resolve the RBF bandwidth: a caller-pinned value, else the median-distance
/// heuristic `σ = √(median{(x_i−x_j)² : i<j, distinct}/2)`. Fails closed when the
/// series is constant (all distances zero ⇒ undefined bandwidth).
fn resolve_bandwidth(name: &str, v: &[f64], pinned: Option<f64>) -> Result<f64> {
    if let Some(s) = pinned {
        if !(s.is_finite() && s > 0.0) {
            return Err(CalyxError::assay_insufficient_samples(format!(
                "HSIC {name} bandwidth must be finite and positive, got {s}"
            )));
        }
        return Ok(s);
    }
    let n = v.len();
    let mut sq = Vec::with_capacity(n * (n - 1) / 2);
    for i in 0..n {
        for j in (i + 1)..n {
            let d = v[i] - v[j];
            if d != 0.0 {
                sq.push(d * d);
            }
        }
    }
    if sq.is_empty() {
        return Err(CalyxError::assay_degenerate_input(format!(
            "HSIC undefined: {name} is constant (zero median distance ⇒ undefined bandwidth)"
        )));
    }
    let med = median(&mut sq);
    Ok((0.5 * med).sqrt())
}

/// Median of a slice (mutates via sort). Non-empty guaranteed by the caller.
fn median(v: &mut [f64]) -> f64 {
    v.sort_by(|a, b| a.partial_cmp(b).expect("finite-validated"));
    let m = v.len();
    if m % 2 == 1 {
        v[m / 2]
    } else {
        0.5 * (v[m / 2 - 1] + v[m / 2])
    }
}

fn to_finite_f64(name: &str, values: &[f32]) -> Result<Vec<f64>> {
    let mut out = Vec::with_capacity(values.len());
    for (idx, &v) in values.iter().enumerate() {
        if !v.is_finite() {
            return Err(CalyxError::assay_insufficient_samples(format!(
                "HSIC {name}[{idx}] is not finite ({v})"
            )));
        }
        out.push(v as f64);
    }
    Ok(out)
}

#[cfg(test)]
#[path = "hsic_tests.rs"]
mod tests;
