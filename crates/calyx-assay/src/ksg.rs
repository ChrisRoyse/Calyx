//! KSG-style k-nearest-neighbor mutual information estimators.

use std::collections::BTreeMap;

use calyx_core::{Anchor, CalyxError, Result};

use crate::bootstrap::{
    BootstrapConfig, DEFAULT_BOOTSTRAP_RESAMPLES, DEFAULT_BOOTSTRAP_SEED,
    bootstrap_mean_ci_with_config, bootstrap_paired_ci,
};
use crate::estimate::{EstimatorKind, MiEstimate, TrustTag, trust_for_anchor};
use crate::samples::validate_rectangular_finite;

pub const MIN_ASSAY_SAMPLES: usize = 50;
const KSG_BOOTSTRAP_CONFIG: BootstrapConfig =
    BootstrapConfig::new(DEFAULT_BOOTSTRAP_RESAMPLES, DEFAULT_BOOTSTRAP_SEED);

pub fn ksg_mi_continuous(x: &[Vec<f32>], y: &[Vec<f32>], k: usize) -> Result<MiEstimate> {
    ksg_mi_continuous_with_trust(x, y, k, TrustTag::Provisional)
}

pub fn ksg_mi_continuous_with_anchor(
    x: &[Vec<f32>],
    y: &[Vec<f32>],
    k: usize,
    anchor: &Anchor,
) -> Result<MiEstimate> {
    ksg_mi_continuous_with_trust(x, y, k, trust_for_anchor(Some(anchor)))
}

fn ksg_mi_continuous_with_trust(
    x: &[Vec<f32>],
    y: &[Vec<f32>],
    k: usize,
    trust: TrustTag,
) -> Result<MiEstimate> {
    validate_samples(x, y, k)?;
    let n = x.len();
    let bits = ksg_bits_from_validated_samples(x, y, k);
    let ci = bootstrap_paired_ci(x, y, bits, KSG_BOOTSTRAP_CONFIG, |sampled_x, sampled_y| {
        Ok(ksg_bits_from_validated_samples(sampled_x, sampled_y, k))
    })?
    .ok_or_else(|| CalyxError::assay_insufficient_samples("bootstrap CI requires samples"))?;
    Ok(MiEstimate::new(
        bits,
        ci.ci_low,
        ci.ci_high,
        n,
        EstimatorKind::Ksg,
        trust,
    ))
}

pub(crate) fn ksg_mi_continuous_point(x: &[Vec<f32>], y: &[Vec<f32>], k: usize) -> Result<f32> {
    validate_samples(x, y, k)?;
    Ok(ksg_bits_from_validated_samples(x, y, k))
}

fn ksg_bits_from_validated_samples(x: &[Vec<f32>], y: &[Vec<f32>], k: usize) -> f32 {
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
    mean(&local_bits).max(0.0)
}

pub fn ksg_mi_continuous_discrete(
    x: &[Vec<f32>],
    labels: &[usize],
    k: usize,
) -> Result<MiEstimate> {
    ksg_mi_continuous_discrete_with_anchor_opt(x, labels, k, None)
}

pub fn ksg_mi_continuous_discrete_with_anchor(
    x: &[Vec<f32>],
    labels: &[usize],
    k: usize,
    anchor: &Anchor,
) -> Result<MiEstimate> {
    ksg_mi_continuous_discrete_with_anchor_opt(x, labels, k, Some(anchor))
}

fn ksg_mi_continuous_discrete_with_anchor_opt(
    x: &[Vec<f32>],
    labels: &[usize],
    k: usize,
    anchor: Option<&Anchor>,
) -> Result<MiEstimate> {
    validate_sample_counts(x.len(), labels.len(), k)?;
    validate_rectangular_finite("x", x)?;
    let class_counts = validate_mixed_discrete_classes(labels, k)?;
    let local_bits = ross_mixed_local_bits_from_validated_samples(x, labels, k, &class_counts);
    let bits = mean(&local_bits).max(0.0);
    let ci = bootstrap_mean_ci_with_config(&local_bits, KSG_BOOTSTRAP_CONFIG)
        .ok_or_else(|| CalyxError::assay_insufficient_samples("bootstrap CI requires samples"))?;
    Ok(MiEstimate::new(
        bits,
        ci.ci_low,
        ci.ci_high,
        x.len(),
        EstimatorKind::Ksg,
        trust_for_anchor(anchor),
    ))
}

fn validate_mixed_discrete_classes(labels: &[usize], k: usize) -> Result<BTreeMap<usize, usize>> {
    let mut counts = BTreeMap::<usize, usize>::new();
    for label in labels {
        *counts.entry(*label).or_default() += 1;
    }
    for (label, count) in &counts {
        if *count <= k {
            return Err(CalyxError::assay_insufficient_samples(format!(
                "mixed continuous-discrete KSG requires at least k+1 samples per discrete label; label={label}, class_size={count}, k={k}, required_min={}",
                k + 1
            )));
        }
    }
    Ok(counts)
}

fn ross_mixed_local_bits_from_validated_samples(
    x: &[Vec<f32>],
    labels: &[usize],
    k: usize,
    class_counts: &BTreeMap<usize, usize>,
) -> Vec<f32> {
    let n = x.len();
    let mut local_bits = Vec::with_capacity(n);
    for i in 0..n {
        let radius = kth_same_class_radius(x, labels, i, k);
        let full_count = neighbor_count_continuous_inclusive(x, i, radius);
        let class_count = class_counts[&labels[i]];
        let local = digamma(n as f64) + digamma(k as f64)
            - digamma(class_count as f64)
            - digamma(full_count as f64);
        local_bits.push((local / std::f64::consts::LN_2) as f32);
    }
    local_bits
}

fn kth_same_class_radius(x: &[Vec<f32>], labels: &[usize], i: usize, k: usize) -> f32 {
    let mut distances = Vec::with_capacity(x.len().saturating_sub(1));
    for j in 0..x.len() {
        if i != j && labels[i] == labels[j] {
            distances.push(chebyshev(&x[i], &x[j]));
        }
    }
    distances.sort_by(f32::total_cmp);
    distances[k - 1]
}

fn neighbor_count_continuous_inclusive(values: &[Vec<f32>], i: usize, radius: f32) -> usize {
    values
        .iter()
        .enumerate()
        .filter(|(j, row)| *j != i && chebyshev(&values[i], row) <= radius)
        .count()
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
