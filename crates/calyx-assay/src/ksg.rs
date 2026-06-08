//! KSG-style k-nearest-neighbor mutual information estimators.

use std::collections::BTreeMap;

use calyx_core::{CalyxError, Result};

use crate::estimate::{EstimatorKind, MiEstimate, TrustTag};
use crate::samples::validate_rectangular_finite;

pub const MIN_ASSAY_SAMPLES: usize = 50;

pub fn ksg_mi_continuous(x: &[Vec<f32>], y: &[Vec<f32>], k: usize) -> Result<MiEstimate> {
    validate_samples(x, y, k)?;
    let n = x.len();
    let mut local_bits = Vec::with_capacity(n);
    for i in 0..n {
        let eps = kth_joint_radius(x, y, i, k);
        let nx = neighbor_count(x, i, eps);
        let ny = neighbor_count(y, i, eps);
        let local = digamma(k as f64) + digamma(n as f64)
            - digamma((nx + 1) as f64)
            - digamma((ny + 1) as f64);
        local_bits.push((local / std::f64::consts::LN_2) as f32);
    }
    let bits = mean(&local_bits).max(0.0);
    let band = ci_band(&local_bits, bits);
    Ok(MiEstimate::new(
        bits,
        bits - band,
        bits + band,
        n,
        EstimatorKind::Ksg,
        TrustTag::Trusted,
    ))
}

pub fn ksg_mi_continuous_discrete(
    x: &[Vec<f32>],
    labels: &[usize],
    k: usize,
) -> Result<MiEstimate> {
    validate_sample_counts(x.len(), labels.len(), k)?;
    validate_rectangular_finite("x", x)?;
    let mut classes = BTreeMap::<usize, usize>::new();
    for label in labels {
        let next = classes.len();
        classes.entry(*label).or_insert(next);
    }
    let y: Vec<Vec<f32>> = labels
        .iter()
        .map(|label| {
            let mut row = vec![0.0; classes.len()];
            row[classes[label]] = 1.0;
            row
        })
        .collect();
    ksg_mi_continuous(x, &y, k)
}

fn validate_samples(x: &[Vec<f32>], y: &[Vec<f32>], k: usize) -> Result<()> {
    validate_sample_counts(x.len(), y.len(), k)?;
    validate_rectangular_finite("x", x)?;
    validate_rectangular_finite("y", y)?;
    Ok(())
}

fn validate_sample_counts(left: usize, right: usize, k: usize) -> Result<()> {
    if left != right || left < MIN_ASSAY_SAMPLES || k == 0 || k >= left {
        return Err(CalyxError::assay_insufficient_samples(format!(
            "need at least {MIN_ASSAY_SAMPLES} paired anchors and 0 < k < n; got left={left}, right={right}, k={k}"
        )));
    }
    Ok(())
}

fn kth_joint_radius(x: &[Vec<f32>], y: &[Vec<f32>], i: usize, k: usize) -> f32 {
    let mut distances = Vec::with_capacity(x.len().saturating_sub(1));
    for j in 0..x.len() {
        if i != j {
            distances.push(chebyshev(&x[i], &x[j]).max(chebyshev(&y[i], &y[j])));
        }
    }
    distances.sort_by(f32::total_cmp);
    distances[k - 1].max(f32::EPSILON)
}

fn neighbor_count(values: &[Vec<f32>], i: usize, radius: f32) -> usize {
    values
        .iter()
        .enumerate()
        .filter(|(j, row)| *j != i && chebyshev(&values[i], row) < radius)
        .count()
}

fn chebyshev(a: &[f32], b: &[f32]) -> f32 {
    a.iter()
        .zip(b)
        .map(|(left, right)| (left - right).abs())
        .fold(0.0, f32::max)
}

fn digamma(mut x: f64) -> f64 {
    let mut result = 0.0;
    while x < 7.0 {
        result -= 1.0 / x;
        x += 1.0;
    }
    let inv = 1.0 / x;
    let inv2 = inv * inv;
    result + x.ln() - 0.5 * inv - inv2 / 12.0 + inv2 * inv2 / 120.0
}

fn mean(values: &[f32]) -> f32 {
    values.iter().sum::<f32>() / values.len() as f32
}

fn ci_band(values: &[f32], bits: f32) -> f32 {
    let avg = mean(values);
    let variance = values
        .iter()
        .map(|value| {
            let delta = value - avg;
            delta * delta
        })
        .sum::<f32>()
        / values.len().saturating_sub(1).max(1) as f32;
    let standard_error = (variance / values.len() as f32).sqrt();
    (1.96 * standard_error).max(bits * 0.20).max(0.05)
}
