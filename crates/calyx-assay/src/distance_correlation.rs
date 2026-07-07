//! Distance correlation (dCor) — an omnibus dependence measure that is **0 iff
//! X and Y are independent** and 1 for an exact linear map (#54). Unlike Pearson
//! or even rank correlation, dCor detects *non-monotone* dependence (e.g. a
//! symmetric `y = x²`), which is exactly the structure a market-signal screen
//! must not miss.
//!
//! Construction (Székely, Rizzo & Bakirov, 2007):
//! 1. Pairwise distance matrices `a_ij = |x_i − x_j|`, `b_ij = |y_i − y_j|`.
//! 2. Double-centre each: `A_ij = a_ij − ā_{i·} − ā_{·j} + ā_{··}`.
//! 3. `dCov²(X,Y) = n⁻² Σ A_ij B_ij`; `dVar²(X) = n⁻² Σ A_ij²`.
//! 4. `dCor²(X,Y) = dCov²(X,Y) / √(dVar²(X)·dVar²(Y))`; `dCor = √dCor²`.
//!
//! Significance is a **permutation test**: the null shuffles Y's sample order
//! (equivalently re-indexes B) with a *seeded, deterministic* RNG and counts how
//! often the permuted `dCov²` reaches the observed value. The p-value uses the
//! add-one estimator `(1 + #ge) / (1 + P)`, which is exact-valid (never 0).
//!
//! Fails closed on length mismatch, non-finite input, `n < 4`, or a constant
//! (zero distance-variance) column — never a silent `NaN`.

use calyx_core::{CalyxError, Result};
use rand::SeedableRng;
use rand::seq::SliceRandom;
use rand_chacha::ChaCha8Rng;
use serde::{Deserialize, Serialize};

/// Minimum paired observations for a defined dCor permutation test.
pub const MIN_DCOR_SAMPLES: usize = 4;
/// Default permutation count for the dCor independence test.
pub const DEFAULT_DCOR_PERMUTATIONS: usize = 999;
/// Default deterministic seed for the permutation null.
pub const DEFAULT_DCOR_SEED: u64 = 0x0D15_7A0C_0DE5_EED5;

/// Distance-correlation point estimate and its components.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct DcorReport {
    pub dcor: f32,
    pub dcov2: f32,
    pub dvar_x: f32,
    pub dvar_y: f32,
    pub n_samples: usize,
}

/// Configuration for the permutation independence test.
#[derive(Clone, Copy, Debug)]
pub struct DcorPermConfig {
    pub permutations: usize,
    pub seed: u64,
}

impl Default for DcorPermConfig {
    fn default() -> Self {
        Self {
            permutations: DEFAULT_DCOR_PERMUTATIONS,
            seed: DEFAULT_DCOR_SEED,
        }
    }
}

/// Distance correlation with a seeded permutation p-value.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct DcorTest {
    pub dcor: f32,
    pub dcov2: f32,
    pub p_value: f32,
    pub permutations: usize,
    /// Count of permutations whose `dCov²` reached the observed value.
    pub ge_count: usize,
    pub seed: u64,
    pub n_samples: usize,
}

/// Distance correlation point estimate over paired samples.
pub fn distance_correlation(x: &[f32], y: &[f32]) -> Result<DcorReport> {
    let (report, _, _) = distance_correlation_inner(x, y)?;
    Ok(report)
}

/// Distance correlation with a permutation-test p-value for independence.
pub fn distance_correlation_test(x: &[f32], y: &[f32], config: DcorPermConfig) -> Result<DcorTest> {
    if config.permutations == 0 {
        return Err(CalyxError::assay_insufficient_samples(
            "dCor permutation test requires permutations > 0",
        ));
    }
    let (report, a, b) = distance_correlation_inner(x, y)?;
    let n = report.n_samples;
    let observed = report.dcov2 as f64;
    // Permuting Y's sample order re-indexes B by the same permutation on rows
    // and columns; A stays fixed. dVar²(X), dVar²(Y) are permutation-invariant,
    // so comparing dCov² is equivalent to comparing dCor.
    let tol = 1e-12 * observed.abs().max(1.0);
    let mut rng = ChaCha8Rng::seed_from_u64(config.seed);
    let mut perm: Vec<usize> = (0..n).collect();
    let mut ge_count = 0usize;
    for _ in 0..config.permutations {
        perm.shuffle(&mut rng);
        let mut acc = 0.0f64;
        for i in 0..n {
            let pi = perm[i] * n;
            let ai = i * n;
            for j in 0..n {
                acc += a[ai + j] * b[pi + perm[j]];
            }
        }
        let dcov2_perm = acc / (n as f64 * n as f64);
        if dcov2_perm >= observed - tol {
            ge_count += 1;
        }
    }
    let p_value = (1.0 + ge_count as f64) / (1.0 + config.permutations as f64);
    Ok(DcorTest {
        dcor: report.dcor,
        dcov2: report.dcov2,
        p_value: p_value as f32,
        permutations: config.permutations,
        ge_count,
        seed: config.seed,
        n_samples: n,
    })
}

/// Shared core: returns the report plus the double-centred distance matrices
/// (row-major, `n×n`) so the permutation test can reuse them.
fn distance_correlation_inner(x: &[f32], y: &[f32]) -> Result<(DcorReport, Vec<f64>, Vec<f64>)> {
    let (xd, yd) = validate_pair(x, y)?;
    let n = xd.len();
    let a = double_centered(&xd);
    let b = double_centered(&yd);
    let dcov2 = mean_product(&a, &b, n).max(0.0);
    let dvar_x = mean_product(&a, &a, n).max(0.0);
    let dvar_y = mean_product(&b, &b, n).max(0.0);
    let denom = (dvar_x * dvar_y).sqrt();
    if denom <= 0.0 {
        return Err(CalyxError::assay_degenerate_input(
            "dCor undefined: a variable has zero distance variance (constant column)",
        ));
    }
    let dcor2 = (dcov2 / denom).clamp(0.0, 1.0);
    let dcor = dcor2.sqrt();
    let report = DcorReport {
        dcor: dcor as f32,
        dcov2: dcov2 as f32,
        dvar_x: dvar_x as f32,
        dvar_y: dvar_y as f32,
        n_samples: n,
    };
    Ok((report, a, b))
}

// ----- numerics --------------------------------------------------------------

fn validate_pair(x: &[f32], y: &[f32]) -> Result<(Vec<f64>, Vec<f64>)> {
    if x.len() != y.len() {
        return Err(CalyxError::assay_insufficient_samples(format!(
            "dCor requires paired samples: x={} y={}",
            x.len(),
            y.len()
        )));
    }
    if x.len() < MIN_DCOR_SAMPLES {
        return Err(CalyxError::assay_insufficient_samples(format!(
            "dCor requires at least {MIN_DCOR_SAMPLES} paired samples; got {}",
            x.len()
        )));
    }
    Ok((to_finite_f64("x", x)?, to_finite_f64("y", y)?))
}

fn to_finite_f64(name: &str, values: &[f32]) -> Result<Vec<f64>> {
    let mut out = Vec::with_capacity(values.len());
    for (idx, &v) in values.iter().enumerate() {
        if !v.is_finite() {
            return Err(CalyxError::assay_insufficient_samples(format!(
                "dCor {name}[{idx}] is not finite ({v})"
            )));
        }
        out.push(v as f64);
    }
    Ok(out)
}

/// Row-major `n×n` double-centred distance matrix of a 1-D sample.
fn double_centered(v: &[f64]) -> Vec<f64> {
    let n = v.len();
    let mut a = vec![0.0f64; n * n];
    for i in 0..n {
        for j in 0..n {
            a[i * n + j] = (v[i] - v[j]).abs();
        }
    }
    // Distance matrices are symmetric, so column mean_j == row mean_j.
    let mut row = vec![0.0f64; n];
    for (i, r) in row.iter_mut().enumerate() {
        let mut s = 0.0;
        for j in 0..n {
            s += a[i * n + j];
        }
        *r = s / n as f64;
    }
    let grand = row.iter().sum::<f64>() / n as f64;
    for i in 0..n {
        for j in 0..n {
            a[i * n + j] = a[i * n + j] - row[i] - row[j] + grand;
        }
    }
    a
}

/// `n⁻² Σ_{ij} p_ij q_ij` over two row-major `n×n` matrices.
fn mean_product(p: &[f64], q: &[f64], n: usize) -> f64 {
    let mut acc = 0.0f64;
    for (a, b) in p.iter().zip(q) {
        acc += a * b;
    }
    acc / (n as f64 * n as f64)
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

    /// Bare Pearson, to demonstrate the cases where dCor sees dependence but
    /// Pearson reports ≈ 0.
    fn pearson(x: &[f32], y: &[f32]) -> f64 {
        let n = x.len() as f64;
        let mx = x.iter().map(|&v| v as f64).sum::<f64>() / n;
        let my = y.iter().map(|&v| v as f64).sum::<f64>() / n;
        let mut cov = 0.0;
        let mut vx = 0.0;
        let mut vy = 0.0;
        for (&a, &b) in x.iter().zip(y) {
            let da = a as f64 - mx;
            let db = b as f64 - my;
            cov += da * db;
            vx += da * da;
            vy += db * db;
        }
        cov / (vx.sqrt() * vy.sqrt())
    }

    #[test]
    fn orthogonal_grid_is_exactly_zero() {
        // X = row, Y = col of a 2×2 grid → independent → dCov² = 0 exactly
        // (hand-derived in the module: every row of A·B sums to 0).
        let x = [0.0f32, 0.0, 1.0, 1.0];
        let y = [0.0f32, 1.0, 0.0, 1.0];
        let r = distance_correlation(&x, &y).unwrap();
        approx(r.dcov2, 0.0, 1e-7, "dCov²");
        approx(r.dcor, 0.0, 1e-6, "dCor");
    }

    #[test]
    fn exact_linear_map_is_one() {
        // dCor = 1 iff Y is a linear function of X.
        let x = [1.0f32, 2.0, 3.0, 4.0, 5.0];
        let y = [2.0f32, 4.0, 6.0, 8.0, 10.0];
        let r = distance_correlation(&x, &y).unwrap();
        approx(r.dcor, 1.0, 1e-6, "dCor(linear)");
    }

    #[test]
    fn catches_nonlinear_dependence_pearson_misses() {
        // Symmetric parabola y = x² over x = ±1..±25 (n = 50). x is symmetric
        // about 0 and y is even, so Σx = Σx³ = 0 → Pearson = 0 *exactly*. The
        // dependence is deterministic; the folded shape keeps the permutation
        // null wide at small n, so we use n = 50 where it rejects decisively.
        let mut x = Vec::new();
        let mut y = Vec::new();
        for k in 1..=25i32 {
            for s in [-1i32, 1] {
                let xi = (s * k) as f32;
                x.push(xi);
                y.push(xi * xi);
            }
        }
        assert!(pearson(&x, &y).abs() < 1e-6, "Pearson must be ~0");
        let t = distance_correlation_test(&x, &y, DcorPermConfig::default()).unwrap();
        assert!(t.dcor > 0.4, "dCor should see the parabola: {t:?}");
        assert!(t.p_value < 0.01, "permutation test should reject: {t:?}");
    }

    #[test]
    fn independent_scatter_is_weak_and_insignificant() {
        let x = [1.0f32, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0];
        let y = [5.0f32, 3.0, 6.0, 2.0, 7.0, 4.0, 8.0, 1.0];
        let t = distance_correlation_test(&x, &y, DcorPermConfig::default()).unwrap();
        assert!(t.p_value > 0.1, "independent → insignificant: {t:?}");
    }

    #[test]
    fn permutation_test_is_deterministic_for_a_seed() {
        let x = [1.0f32, 2.0, 3.0, 4.0, 5.0, 6.0];
        let y = [1.0f32, 4.0, 9.0, 16.0, 25.0, 36.0];
        let cfg = DcorPermConfig {
            permutations: 500,
            seed: 12345,
        };
        let a = distance_correlation_test(&x, &y, cfg).unwrap();
        let b = distance_correlation_test(&x, &y, cfg).unwrap();
        assert_eq!(a.p_value, b.p_value, "same seed → same p-value");
        assert_eq!(a.ge_count, b.ge_count);
    }

    #[test]
    fn fails_closed_on_bad_input() {
        assert_eq!(
            distance_correlation(&[1.0, 2.0, 3.0], &[1.0, 2.0])
                .unwrap_err()
                .code,
            "CALYX_ASSAY_INSUFFICIENT_SAMPLES"
        );
        assert_eq!(
            distance_correlation(&[1.0, 2.0, 3.0], &[1.0, 2.0, 3.0])
                .unwrap_err()
                .code,
            "CALYX_ASSAY_INSUFFICIENT_SAMPLES" // n < 4
        );
        assert_eq!(
            distance_correlation(&[1.0, f32::NAN, 3.0, 4.0], &[1.0, 2.0, 3.0, 4.0])
                .unwrap_err()
                .code,
            "CALYX_ASSAY_INSUFFICIENT_SAMPLES"
        );
    }

    #[test]
    fn fails_closed_on_constant_column() {
        let e = distance_correlation(&[5.0, 5.0, 5.0, 5.0], &[1.0, 2.0, 3.0, 4.0]).unwrap_err();
        assert_eq!(e.code, "CALYX_ASSAY_DEGENERATE_INPUT");
    }
}
