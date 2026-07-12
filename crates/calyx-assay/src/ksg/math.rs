//! Numerical helpers shared by the continuous and mixed KSG estimators.

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
