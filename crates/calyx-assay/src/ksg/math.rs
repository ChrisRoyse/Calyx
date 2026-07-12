//! Numerical helpers shared by the continuous and mixed KSG estimators.

use calyx_core::{CalyxError, Result};

pub(super) fn percentile_index(len: usize, p: f32) -> usize {
    let last = len.saturating_sub(1);
    ((last as f32 * p).round() as usize).min(last)
}

pub(super) fn kth_distance(distances: &mut [f32], k: usize) -> &f32 {
    let (_, kth, _) = distances.select_nth_unstable_by(k - 1, |a, b| a.total_cmp(b));
    kth
}

pub(super) fn chebyshev(a: &[f32], b: &[f32]) -> f32 {
    a.iter()
        .zip(b)
        .map(|(left, right)| (left - right).abs())
        .fold(0.0, f32::max)
}

pub(super) fn validate_finite_chebyshev_domain(name: &str, samples: &[Vec<f32>]) -> Result<()> {
    for dimension in 0..samples[0].len() {
        let mut min = f32::INFINITY;
        let mut max = f32::NEG_INFINITY;
        for row in samples {
            min = min.min(row[dimension]);
            max = max.max(row[dimension]);
        }
        if !(max - min).is_finite() {
            return Err(CalyxError::assay_degenerate_input(format!(
                "{name} dimension {dimension} cannot produce a finite f32 Chebyshev distance: min={min} max={max}"
            )));
        }
    }
    Ok(())
}

pub(super) fn digamma(mut x: f64) -> f64 {
    let mut result = 0.0;
    while x < 7.0 {
        result -= 1.0 / x;
        x += 1.0;
    }
    let inv = 1.0 / x;
    let inv2 = inv * inv;
    result + x.ln() - 0.5 * inv - inv2 / 12.0 + inv2 * inv2 / 120.0
}

pub(super) fn mean(values: &[f32]) -> f32 {
    values.iter().sum::<f32>() / values.len() as f32
}
