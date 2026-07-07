//! MIC — the Maximal Information Coefficient (Reshef et al., *Science* 2011), an
//! *equitable* dependence measure in `[0, 1]` that scores linear, non-linear,
//! monotone, and non-monotone (periodic, parabolic) relationships on a common
//! scale (#56). Where the KSG mutual-information estimator returns unbounded
//! nats/bits, MIC returns a normalized, human-readable strength that is directly
//! comparable across relationship shapes — a bounded edge weight for the
//! redundancy graph and a market-signal screen that must not miss non-monotone
//! structure.
//!
//! Definition: over grids `G` with `nx·ny ≤ B(n)`,
//! `MIC = max_G  I_G(X;Y) / log₂(min(nx, ny))`, where `I_G` is the mutual
//! information (base 2) of the empirical distribution on the grid and the budget
//! is `B(n) = max(n^α, 4)` with `α = 0.6` by default.
//!
//! This computes the standard **ApproxMaxMI** characteristic matrix, not the
//! (intractable) exact max over all grids: for each bin count on one axis it
//! *equipartitions* that axis (equal-frequency, ties kept together) and finds the
//! mutual-information-maximizing partition of the other axis by **dynamic
//! programming** (Reshef Alg. 2 — the optimum lies on value boundaries, and the
//! DP searches them exactly). Both orientations are run and the per-cell maximum
//! taken, exactly as `minepy`. A noiseless functional relationship scores 1;
//! independence tends to 0 (with the known small-sample upward bias).
//!
//! Fails closed on length mismatch, non-finite input, `n < 4`, or a constant
//! (single-distinct-value) column — for which the grid degenerates and MIC is
//! undefined — never a silent `NaN`.

use calyx_core::{CalyxError, Result};
use serde::{Deserialize, Serialize};

/// Default grid-budget exponent `α` in `B(n) = max(n^α, 4)` (Reshef 2011).
pub const DEFAULT_MIC_ALPHA: f64 = 0.6;
/// Minimum samples for a defined MIC (need a 2×2 grid, i.e. `B(n) ≥ 4`).
pub const MIN_MIC_SAMPLES: usize = 4;

/// MIC point estimate and the winning grid resolution.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct MicReport {
    pub mic: f32,
    /// Number of x-bins in the maximizing grid.
    pub best_nx: usize,
    /// Number of y-bins in the maximizing grid.
    pub best_ny: usize,
    /// Grid-cell budget `B(n) = max(n^α, 4)`.
    pub b_budget: usize,
    pub n_samples: usize,
}

/// MIC with the default budget exponent `α = 0.6`.
pub fn mic(x: &[f32], y: &[f32]) -> Result<MicReport> {
    mic_with_alpha(x, y, DEFAULT_MIC_ALPHA)
}

/// MIC with an explicit budget exponent `α ∈ (0, 1]` (Anneal-tunable).
pub fn mic_with_alpha(x: &[f32], y: &[f32], alpha: f64) -> Result<MicReport> {
    if !(alpha.is_finite() && alpha > 0.0 && alpha <= 1.0) {
        return Err(CalyxError::assay_insufficient_samples(format!(
            "MIC budget exponent α must be in (0, 1]; got {alpha}"
        )));
    }
    if x.len() != y.len() {
        return Err(CalyxError::assay_insufficient_samples(format!(
            "MIC requires paired samples: x={} y={}",
            x.len(),
            y.len()
        )));
    }
    let n = x.len();
    if n < MIN_MIC_SAMPLES {
        return Err(CalyxError::assay_insufficient_samples(format!(
            "MIC requires at least {MIN_MIC_SAMPLES} paired samples; got {n}"
        )));
    }
    let xd = to_finite_f64("x", x)?;
    let yd = to_finite_f64("y", y)?;
    if distinct_count(&xd) < 2 || distinct_count(&yd) < 2 {
        return Err(CalyxError::assay_degenerate_input(
            "MIC undefined: a variable is constant (a single distinct value ⇒ no grid)",
        ));
    }

    let b_budget = (n as f64).powf(alpha).floor().max(4.0) as usize;

    // Orientation 1: equipartition Y (secondary), DP-optimize X (primary).
    let (mic1, nx1, ny1) = scan(&xd, &yd, b_budget);
    // Orientation 2: equipartition X (secondary), DP-optimize Y (primary).
    let (mic2, ny2, nx2) = scan(&yd, &xd, b_budget);

    let (mic_val, best_nx, best_ny) = if mic1 >= mic2 {
        (mic1, nx1, ny1)
    } else {
        (mic2, nx2, ny2)
    };
    let mic_val = mic_val.clamp(0.0, 1.0);

    Ok(MicReport {
        mic: mic_val as f32,
        best_nx,
        best_ny,
        b_budget,
        n_samples: n,
    })
}

// ----- ApproxMaxMI core ------------------------------------------------------

/// Scan every secondary-axis resolution: equipartition the `secondary` axis into
/// `ny` bins and DP-optimize the `primary` axis into `nx` bins for all
/// `nx·ny ≤ B`. Returns the best normalized score and the `(n_primary, n_secondary)`
/// bin counts that achieved it.
fn scan(primary: &[f64], secondary: &[f64], b_budget: usize) -> (f64, usize, usize) {
    let n = primary.len();
    // Sort point indices by the primary axis.
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&a, &b| {
        primary[a]
            .partial_cmp(&primary[b])
            .expect("finite-validated")
    });
    let primary_sorted: Vec<f64> = order.iter().map(|&i| primary[i]).collect();

    // Boundaries between distinct primary values are the only valid x-cuts.
    let n_primary_groups = 1
        + (1..n)
            .filter(|&p| primary_sorted[p] != primary_sorted[p - 1])
            .count();

    let mut best = 0.0f64;
    let mut best_np = 2usize;
    let mut best_ns = 2usize;

    let max_secondary = b_budget / 2; // need at least nx = 2
    for ny in 2..=max_secondary {
        // Equipartition the secondary axis into ny equal-frequency bins.
        let sec_bins = equipartition(secondary, ny);
        let actual_ny = sec_bins.iter().copied().max().map(|m| m + 1).unwrap_or(1);
        if actual_ny < 2 {
            continue; // ties collapsed the axis; a finer request cannot help
        }
        // Secondary bins for points in primary-sorted order.
        let sec_sorted: Vec<usize> = order.iter().map(|&i| sec_bins[i]).collect();

        let max_primary = (b_budget / ny).min(n_primary_groups);
        if max_primary < 2 {
            continue;
        }
        // DP: best mutual information (bits) for each primary-bin count 2..=max_primary.
        let mi = optimize_axis(&primary_sorted, &sec_sorted, actual_ny, max_primary);
        for (nx, &mi_nx) in mi.iter().enumerate().skip(2) {
            let denom = (nx.min(actual_ny) as f64).log2();
            if denom <= 0.0 {
                continue;
            }
            let score = mi_nx / denom;
            if score > best {
                best = score;
                best_np = nx;
                best_ns = actual_ny;
            }
        }
    }
    (best, best_np, best_ns)
}

/// DP over the primary axis (Reshef OptimizeXAxis): given points in primary-sorted
/// order with their fixed secondary-bin labels, return `mi[l]` = the maximum grid
/// mutual information (bits) achievable with exactly `l` primary bins, for
/// `l = 0..=max_primary`. Cuts are permitted only between distinct primary values.
// The DP is inherently index-based (each `i` addresses `gs`, `cut_allowed`, and the
// `wh(i, m)` range entropy), so a range loop is the clearest form here.
#[allow(clippy::needless_range_loop)]
fn optimize_axis(
    primary_sorted: &[f64],
    sec_sorted: &[usize],
    n_secondary: usize,
    max_primary: usize,
) -> Vec<f64> {
    let n = sec_sorted.len();
    // Cumulative secondary-bin counts: cum[m*S + r] = #points in sorted[0..m] in bin r.
    let s = n_secondary;
    let mut cum = vec![0u32; (n + 1) * s];
    for m in 0..n {
        for r in 0..s {
            cum[(m + 1) * s + r] = cum[m * s + r];
        }
        cum[(m + 1) * s + sec_sorted[m]] += 1;
    }
    // Entropy H(secondary) over all n points (bits).
    let h_secondary = {
        let nf = n as f64;
        let mut h = 0.0f64;
        for r in 0..s {
            let c = cum[n * s + r] as f64;
            if c > 0.0 {
                let p = c / nf;
                h -= p * p.log2();
            }
        }
        h
    };

    // WH(i, m) = (m−i)·H(secondary | points in (i, m]) in bits, weighted (not averaged).
    // = (m−i)·log₂(m−i) − Σ_r c_r·log₂(c_r),  c_r = cum[m][r] − cum[i][r].
    let wh = |i: usize, m: usize| -> f64 {
        let total = (m - i) as f64;
        let mut acc = total * total.log2();
        for r in 0..s {
            let c = (cum[m * s + r] - cum[i * s + r]) as f64;
            if c > 0.0 {
                acc -= c * c.log2();
            }
        }
        acc
    };

    // Valid cut positions: i in 1..n with a distinct primary value across the seam.
    let cut_allowed: Vec<bool> = (0..=n)
        .map(|i| i >= 1 && i < n && primary_sorted[i] != primary_sorted[i - 1])
        .collect();

    // GS[m][l] = min Σ_cols n_c·H_c (weighted sum, bits) over l-column partitions of
    // the first m points. Then MI(l) = H(secondary) − GS[n][l]/n.
    const INF: f64 = f64::INFINITY;
    let mut gs = vec![INF; (n + 1) * (max_primary + 1)];
    // l = 1: a single column over the first m points.
    for m in 1..=n {
        gs[m * (max_primary + 1) + 1] = wh(0, m);
    }
    for l in 2..=max_primary {
        for m in l..=n {
            // Last column spans (i, m]; the first i points hold l−1 columns.
            // i must be a valid cut and leave ≥ l−1 points for l−1 columns.
            let mut best = INF;
            let base = |i: usize| gs[i * (max_primary + 1) + (l - 1)];
            for i in (l - 1)..m {
                if !cut_allowed[i] {
                    continue;
                }
                let prev = base(i);
                if prev == INF {
                    continue;
                }
                let cand = prev + wh(i, m);
                if cand < best {
                    best = cand;
                }
            }
            gs[m * (max_primary + 1) + l] = best;
        }
    }

    let nf = n as f64;
    let mut mi = vec![0.0f64; max_primary + 1];
    for l in 1..=max_primary {
        let g = gs[n * (max_primary + 1) + l];
        mi[l] = if g.is_finite() {
            (h_secondary - g / nf).max(0.0)
        } else {
            0.0
        };
    }
    mi
}

/// Equal-frequency partition of `values` into `k` bins, keeping equal values in
/// the same bin. Returns a per-point bin index in `0..actual_bins` (which may be
/// fewer than `k` under heavy ties or few distinct values).
fn equipartition(values: &[f64], k: usize) -> Vec<usize> {
    let n = values.len();
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&a, &b| values[a].partial_cmp(&values[b]).expect("finite-validated"));

    // Ideal cut positions in sorted order, snapped to value-change boundaries.
    let mut boundaries: Vec<usize> = Vec::new();
    for b in 1..k {
        let ideal = ((b as f64) * (n as f64) / (k as f64)).round() as usize;
        match snap_to_value_boundary(&order, values, ideal) {
            Some(pos) if boundaries.last() != Some(&pos) => boundaries.push(pos),
            _ => {}
        }
    }

    // Assign bins in sorted order, incrementing at each boundary.
    let mut bin_of_sorted = vec![0usize; n];
    let mut cur = 0usize;
    let mut bptr = 0usize;
    for (rank, slot) in bin_of_sorted.iter_mut().enumerate() {
        while bptr < boundaries.len() && rank == boundaries[bptr] {
            cur += 1;
            bptr += 1;
        }
        *slot = cur;
    }
    // Scatter back to original point order.
    let mut out = vec![0usize; n];
    for (rank, &orig) in order.iter().enumerate() {
        out[orig] = bin_of_sorted[rank];
    }
    out
}

/// Snap a desired sorted-position cut to the nearest position where the value
/// actually changes (so a tie group is never split). `None` if no interior
/// boundary exists near `ideal`.
fn snap_to_value_boundary(order: &[usize], values: &[f64], ideal: usize) -> Option<usize> {
    let n = order.len();
    let is_boundary = |p: usize| p >= 1 && p < n && values[order[p]] != values[order[p - 1]];
    if ideal >= 1 && ideal < n && is_boundary(ideal) {
        return Some(ideal);
    }
    // Search outward for the closest value-change boundary.
    for delta in 1..n {
        let up = ideal + delta;
        if up < n && is_boundary(up) {
            return Some(up);
        }
        if ideal >= delta {
            let down = ideal - delta;
            if is_boundary(down) {
                return Some(down);
            }
        }
    }
    None
}

fn distinct_count(v: &[f64]) -> usize {
    let mut s = v.to_vec();
    s.sort_by(|a, b| a.partial_cmp(b).expect("finite-validated"));
    s.dedup();
    s.len()
}

fn to_finite_f64(name: &str, values: &[f32]) -> Result<Vec<f64>> {
    let mut out = Vec::with_capacity(values.len());
    for (idx, &v) in values.iter().enumerate() {
        if !v.is_finite() {
            return Err(CalyxError::assay_insufficient_samples(format!(
                "MIC {name}[{idx}] is not finite ({v})"
            )));
        }
        out.push(v as f64);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noiseless_bijection_is_one() {
        // y = x, strictly increasing bijection → MIC = 1 exactly.
        let n = 40;
        let x: Vec<f32> = (0..n).map(|i| i as f32).collect();
        let y = x.clone();
        let r = mic(&x, &y).unwrap();
        assert!((r.mic - 1.0).abs() < 1e-6, "bijection MIC=1: {r:?}");
    }

    #[test]
    fn strictly_decreasing_is_one() {
        let n = 40;
        let x: Vec<f32> = (0..n).map(|i| i as f32).collect();
        let y: Vec<f32> = (0..n).map(|i| (n - i) as f32).collect();
        let r = mic(&x, &y).unwrap();
        assert!((r.mic - 1.0).abs() < 1e-6, "decreasing MIC=1: {r:?}");
    }

    #[test]
    fn noiseless_parabola_is_one() {
        // y = x² over symmetric distinct x: a noiseless (non-monotone) function
        // of x → MIC = 1. Pearson would be ~0; MIC sees it.
        let x: Vec<f32> = (-20..=20).map(|i| i as f32).collect();
        let y: Vec<f32> = x.iter().map(|&v| v * v).collect();
        let r = mic(&x, &y).unwrap();
        assert!(r.mic > 0.99, "parabola MIC≈1: {r:?}");
    }

    #[test]
    fn independent_is_small() {
        // Deterministic independent scatter → MIC well below 1 (small-sample bias
        // keeps it > 0, so we bound it loosely).
        let n = 200usize;
        let x: Vec<f32> = (0..n).map(|i| splitmix(i as u64) as f32).collect();
        let y: Vec<f32> = (0..n).map(|i| splitmix(5000 + i as u64) as f32).collect();
        let r = mic(&x, &y).unwrap();
        assert!(r.mic < 0.5, "independent MIC should be small: {r:?}");
    }

    #[test]
    fn dependent_beats_independent() {
        // A clear (noisy) monotone relation should score higher than independence.
        let n = 200usize;
        let x: Vec<f32> = (0..n).map(|i| splitmix(i as u64) as f32).collect();
        let y_dep: Vec<f32> = x
            .iter()
            .enumerate()
            .map(|(i, &v)| v + 0.05 * (splitmix(9000 + i as u64) as f32 - 0.5))
            .collect();
        let y_ind: Vec<f32> = (0..n).map(|i| splitmix(5000 + i as u64) as f32).collect();
        let dep = mic(&x, &y_dep).unwrap();
        let ind = mic(&x, &y_ind).unwrap();
        assert!(
            dep.mic > ind.mic + 0.2,
            "dependent > independent: dep={dep:?} ind={ind:?}"
        );
        assert!(dep.mic > 0.7, "strong relation scores high: {dep:?}");
    }

    #[test]
    fn mic_is_bounded_unit_interval() {
        let n = 60usize;
        let x: Vec<f32> = (0..n).map(|i| splitmix(i as u64) as f32).collect();
        let y: Vec<f32> = (0..n)
            .map(|i| (splitmix(i as u64) * 3.0).sin() as f32)
            .collect();
        let r = mic(&x, &y).unwrap();
        assert!((0.0..=1.0).contains(&r.mic), "MIC in [0,1]: {r:?}");
    }

    #[test]
    fn fails_closed_on_bad_input() {
        assert_eq!(
            mic(&[1.0, 2.0, 3.0], &[1.0, 2.0]).unwrap_err().code,
            "CALYX_ASSAY_INSUFFICIENT_SAMPLES"
        );
        assert_eq!(
            mic(&[1.0, 2.0, 3.0], &[1.0, 2.0, 3.0]).unwrap_err().code,
            "CALYX_ASSAY_INSUFFICIENT_SAMPLES" // n < 4
        );
        assert_eq!(
            mic(&[1.0, f32::NAN, 3.0, 4.0], &[1.0, 2.0, 3.0, 4.0])
                .unwrap_err()
                .code,
            "CALYX_ASSAY_INSUFFICIENT_SAMPLES"
        );
    }

    #[test]
    fn fails_closed_on_constant_column() {
        let e = mic(&[5.0, 5.0, 5.0, 5.0, 5.0], &[1.0, 2.0, 3.0, 4.0, 5.0]).unwrap_err();
        assert_eq!(e.code, "CALYX_ASSAY_DEGENERATE_INPUT");
    }

    #[test]
    fn fails_closed_on_bad_alpha() {
        let e = mic_with_alpha(&[1.0, 2.0, 3.0, 4.0], &[1.0, 2.0, 3.0, 4.0], 1.5).unwrap_err();
        assert_eq!(e.code, "CALYX_ASSAY_INSUFFICIENT_SAMPLES");
    }

    /// Deterministic splitmix64 → uniform f64 in [0,1); reproducible, no RNG.
    fn splitmix(mut x: u64) -> f64 {
        x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = x;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        ((z >> 11) as f64) / ((1_u64 << 53) as f64)
    }
}
