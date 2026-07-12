//! Ross mixed continuous-discrete KSG point and subsampling-root interval.

use std::collections::{BTreeMap, BTreeSet};

use calyx_core::{CalyxError, Result};
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;

use super::math::{chebyshev, digamma, kth_distance, percentile_index};
use crate::bootstrap::{BootstrapCi, BootstrapConfig, DEFAULT_BOOTSTRAP_SEED};
use crate::estimate::{EstimateBound, EstimatorKind, MiEstimate, TrustTag};
use crate::special_fn::ln_gamma;
use crate::subsample::sample_without_replacement_indices;

pub(super) const MIXED_KSG_SUBSAMPLE_RESAMPLES: usize = 999;
const MIXED_KSG_CI_CONFIG: BootstrapConfig =
    BootstrapConfig::new(MIXED_KSG_SUBSAMPLE_RESAMPLES, DEFAULT_BOOTSTRAP_SEED);
const MAX_SUPPORT_FAILURE: f64 = 0.01;
const MAX_DRAW_ATTEMPTS: usize = 8;

#[derive(Clone, Copy, Debug)]
pub(super) struct MixedSubsamplePlan {
    pub(super) m: usize,
    pub(super) support_failure_upper: f64,
}

#[derive(Clone, Debug)]
pub(super) struct MixedKsgCi {
    pub(super) interval: BootstrapCi,
    pub(super) plan: MixedSubsamplePlan,
    pub(super) rejected_draws: usize,
}

pub(super) fn estimate(
    x: &[Vec<f32>],
    labels: &[usize],
    k: usize,
    trust: TrustTag,
) -> Result<MiEstimate> {
    let class_counts = validate_classes(labels, k)?;
    validate_radius_defined(x, labels, k)?;
    let raw_bits = raw_bits_from_validated_samples(x, labels, k, &class_counts);
    let ci = subsample_ci(x, labels, raw_bits, k, MIXED_KSG_CI_CONFIG)?;
    finalize_estimate(raw_bits, ci, x.len(), trust)
}

pub(super) fn validate_classes(labels: &[usize], k: usize) -> Result<BTreeMap<usize, usize>> {
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

pub(super) fn validate_radius_defined(x: &[Vec<f32>], labels: &[usize], k: usize) -> Result<()> {
    for i in 0..x.len() {
        let exact_duplicates = (0..x.len())
            .filter(|&j| i != j && labels[i] == labels[j] && chebyshev(&x[i], &x[j]) == 0.0)
            .count();
        if exact_duplicates >= k {
            return Err(CalyxError::assay_degenerate_input(format!(
                "mixed continuous-discrete KSG kth same-class radius is zero for sample {i}: label={} exact_same_class_duplicates={exact_duplicates} k={k}",
                labels[i]
            )));
        }
    }
    Ok(())
}

pub(super) fn raw_bits_from_validated_samples(
    x: &[Vec<f32>],
    labels: &[usize],
    k: usize,
    class_counts: &BTreeMap<usize, usize>,
) -> f64 {
    let n = x.len();
    let mut total = 0.0;
    for i in 0..n {
        let (radius, same_class_count) = kth_same_class_radius_and_count(x, labels, i, k);
        let full_count = neighbor_count_inclusive(x, i, radius);
        let class_count = class_counts[&labels[i]];
        total += digamma(n as f64) + digamma(same_class_count as f64)
            - digamma(class_count as f64)
            - digamma(full_count as f64);
    }
    total / n as f64 / std::f64::consts::LN_2
}

fn kth_same_class_radius_and_count(
    x: &[Vec<f32>],
    labels: &[usize],
    i: usize,
    k: usize,
) -> (f32, usize) {
    let mut distances = Vec::with_capacity(x.len().saturating_sub(1));
    for j in 0..x.len() {
        if i != j && labels[i] == labels[j] {
            distances.push(chebyshev(&x[i], &x[j]));
        }
    }
    let radius = *kth_distance(&mut distances, k);
    let same_class_count = distances
        .iter()
        .filter(|distance| **distance <= radius)
        .count();
    (radius, same_class_count)
}

fn neighbor_count_inclusive(values: &[Vec<f32>], i: usize, radius: f32) -> usize {
    values
        .iter()
        .enumerate()
        .filter(|(j, row)| *j != i && chebyshev(&values[i], row) <= radius)
        .count()
}

pub(super) fn subsample_ci(
    x: &[Vec<f32>],
    labels: &[usize],
    raw_point_estimate: f64,
    k: usize,
    config: BootstrapConfig,
) -> Result<MixedKsgCi> {
    if config.resamples == 0 {
        return Err(CalyxError::assay_insufficient_samples(
            "mixed continuous-discrete KSG no-replacement CI requires at least one resample",
        ));
    }
    if !raw_point_estimate.is_finite() {
        return Err(CalyxError::assay_low_signal(
            "mixed continuous-discrete KSG full-sample estimate is non-finite",
        ));
    }
    let full_counts = validate_classes(labels, k)?;
    let plan = subsample_plan(x.len(), k, &full_counts)?;
    let mut rng = ChaCha8Rng::seed_from_u64(config.seed);
    let mut roots = Vec::with_capacity(config.resamples);
    let mut rejected_draws = 0usize;
    let root_scale = finite_population_root_scale(x.len(), plan.m)?;
    for _ in 0..config.resamples {
        let (indices, rejected) =
            sample_indices_with_rejections(labels, plan.m, k, MAX_DRAW_ATTEMPTS, &mut rng)?;
        rejected_draws = rejected_draws.saturating_add(rejected);
        let sampled_x = indices
            .iter()
            .map(|index| x[*index].clone())
            .collect::<Vec<_>>();
        let sampled_labels = indices
            .iter()
            .map(|index| labels[*index])
            .collect::<Vec<_>>();
        let sampled_counts = validate_classes(&sampled_labels, k)?;
        let sampled_raw =
            raw_bits_from_validated_samples(&sampled_x, &sampled_labels, k, &sampled_counts);
        let root = root_scale * (sampled_raw - raw_point_estimate);
        if !sampled_raw.is_finite() || !root.is_finite() {
            return Err(CalyxError::assay_low_signal(
                "mixed continuous-discrete KSG produced a non-finite subsampling root",
            ));
        }
        roots.push(root);
    }
    Ok(MixedKsgCi {
        interval: ci_from_roots(roots, raw_point_estimate, x.len())?,
        plan,
        rejected_draws,
    })
}

pub(super) fn finite_population_root_scale(n: usize, m: usize) -> Result<f64> {
    if n == 0 || m == 0 || m >= n {
        return Err(CalyxError::assay_insufficient_samples(format!(
            "mixed continuous-discrete KSG finite-population root requires 0 < m < n; n={n}, m={m}"
        )));
    }
    let finite_population = 1.0 - m as f64 / n as f64;
    let root_scale = (m as f64 / finite_population).sqrt();
    if !root_scale.is_finite() {
        return Err(CalyxError::assay_low_signal(
            "mixed continuous-discrete KSG finite-population root scale is non-finite",
        ));
    }
    Ok(root_scale)
}

#[cfg(test)]
pub(super) fn sample_mixed_indices<R: Rng + ?Sized>(
    labels: &[usize],
    m: usize,
    k: usize,
    rng: &mut R,
) -> Result<Vec<usize>> {
    sample_indices_with_rejections(labels, m, k, MAX_DRAW_ATTEMPTS, rng).map(|(indices, _)| indices)
}

fn sample_indices_with_rejections<R: Rng + ?Sized>(
    labels: &[usize],
    m: usize,
    k: usize,
    max_attempts: usize,
    rng: &mut R,
) -> Result<(Vec<usize>, usize)> {
    if m == 0 || m > labels.len() {
        return Err(CalyxError::assay_insufficient_samples(format!(
            "mixed continuous-discrete KSG no-replacement subsample requires 0 < m <= n; got n={}, m={m}",
            labels.len()
        )));
    }
    if max_attempts == 0 {
        return Err(CalyxError::assay_insufficient_samples(
            "mixed continuous-discrete KSG class-support sampler requires at least one attempt",
        ));
    }
    let class_total = labels.iter().copied().collect::<BTreeSet<_>>().len();
    for attempt in 0..max_attempts {
        let indices = sample_without_replacement_indices(labels.len(), m, rng)?;
        let mut counts = BTreeMap::<usize, usize>::new();
        for index in &indices {
            *counts.entry(labels[*index]).or_default() += 1;
        }
        if counts.len() == class_total && counts.values().all(|count| *count > k) {
            return Ok((indices, attempt));
        }
    }
    Err(CalyxError::assay_insufficient_samples(format!(
        "mixed continuous-discrete KSG class-support rejection exhausted {max_attempts} uniform draws; n={}, m={m}, k={k}",
        labels.len()
    )))
}

pub(super) fn subsample_plan(
    n: usize,
    k: usize,
    class_counts: &BTreeMap<usize, usize>,
) -> Result<MixedSubsamplePlan> {
    let m_min = ((n as f64).powf(2.0 / 3.0).ceil() as usize).max(k.saturating_add(2));
    let m_max = n / 2;
    if m_min > m_max {
        return Err(CalyxError::assay_insufficient_samples(format!(
            "mixed continuous-discrete KSG class-support CI has no half-sample range; n={n}, k={k}, m_min={m_min}, m_max={m_max}"
        )));
    }
    for m in m_min..=m_max {
        let mut support_failure_upper = 0.0;
        for count in class_counts.values() {
            support_failure_upper += hypergeometric_cdf_at_most(n, *count, m, k)?;
        }
        if !support_failure_upper.is_finite() {
            return Err(CalyxError::assay_low_signal(
                "mixed continuous-discrete KSG class-support probability is non-finite",
            ));
        }
        support_failure_upper = support_failure_upper.min(1.0);
        if support_failure_upper <= MAX_SUPPORT_FAILURE {
            return Ok(MixedSubsamplePlan {
                m,
                support_failure_upper,
            });
        }
    }
    Err(CalyxError::assay_insufficient_samples(format!(
        "mixed continuous-discrete KSG class-support CI found no m in {m_min}..={m_max} with invalid-subsample probability <= {MAX_SUPPORT_FAILURE:.4}; n={n}, k={k}, class_counts={class_counts:?}"
    )))
}

pub(super) fn hypergeometric_cdf_at_most(
    population: usize,
    successes: usize,
    draws: usize,
    threshold: usize,
) -> Result<f64> {
    if successes > population || draws > population {
        return Err(CalyxError::assay_insufficient_samples(format!(
            "mixed continuous-discrete KSG hypergeometric domain requires successes,draws <= population; population={population}, successes={successes}, draws={draws}"
        )));
    }
    let first = draws.saturating_sub(population - successes);
    let last = threshold.min(successes).min(draws);
    if first > last {
        return Ok(0.0);
    }
    let denominator = ln_binomial(population, draws);
    let mut log_terms = Vec::with_capacity(last - first + 1);
    for selected in first..=last {
        log_terms.push(
            ln_binomial(successes, selected)
                + ln_binomial(population - successes, draws - selected)
                - denominator,
        );
    }
    let max_log = log_terms.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let probability = max_log.exp()
        * log_terms
            .iter()
            .map(|term| (*term - max_log).exp())
            .sum::<f64>();
    if !probability.is_finite() || !(-1e-12..=1.0 + 1e-9).contains(&probability) {
        return Err(CalyxError::assay_low_signal(format!(
            "mixed continuous-discrete KSG hypergeometric probability is invalid: {probability}"
        )));
    }
    Ok(probability.clamp(0.0, 1.0))
}

fn ln_binomial(n: usize, k: usize) -> f64 {
    if k > n {
        return f64::NEG_INFINITY;
    }
    ln_gamma((n + 1) as f64) - ln_gamma((k + 1) as f64) - ln_gamma((n - k + 1) as f64)
}

pub(super) fn ci_from_roots(
    mut roots: Vec<f64>,
    raw_point_estimate: f64,
    n: usize,
) -> Result<BootstrapCi> {
    if roots.is_empty() || n == 0 || !raw_point_estimate.is_finite() {
        return Err(CalyxError::assay_insufficient_samples(
            "mixed continuous-discrete KSG root CI requires finite point evidence and roots",
        ));
    }
    if roots.iter().any(|root| !root.is_finite()) {
        return Err(CalyxError::assay_low_signal(
            "mixed continuous-discrete KSG root CI contains non-finite evidence",
        ));
    }
    roots.sort_by(f64::total_cmp);
    let low_root = roots[percentile_index(roots.len(), 0.025)];
    let high_root = roots[percentile_index(roots.len(), 0.975)];
    let full_scale = (n as f64).sqrt();
    let ci_low = raw_point_estimate - high_root / full_scale;
    let ci_high = raw_point_estimate - low_root / full_scale;
    if !ci_low.is_finite() || !ci_high.is_finite() || ci_low > ci_high {
        return Err(CalyxError::assay_low_signal(format!(
            "mixed continuous-discrete KSG root CI is invalid: point={raw_point_estimate}, low={ci_low}, high={ci_high}"
        )));
    }
    let mean = raw_point_estimate as f32;
    let low = ci_low as f32;
    let high = ci_high as f32;
    if !mean.is_finite() || !low.is_finite() || !high.is_finite() || low > high {
        return Err(CalyxError::assay_low_signal(
            "mixed continuous-discrete KSG root CI cannot be represented as finite f32 evidence",
        ));
    }
    Ok(BootstrapCi {
        mean,
        ci_low: low,
        ci_high: high,
        resamples: roots.len(),
    })
}

fn finalize_estimate(
    raw_point_estimate: f64,
    ci: MixedKsgCi,
    n_samples: usize,
    trust: TrustTag,
) -> Result<MiEstimate> {
    if ci.interval.resamples != MIXED_KSG_SUBSAMPLE_RESAMPLES
        || ci.plan.m == 0
        || ci.plan.m > n_samples / 2
        || !ci.plan.support_failure_upper.is_finite()
        || ci.plan.support_failure_upper > MAX_SUPPORT_FAILURE
        || ci.rejected_draws > MIXED_KSG_SUBSAMPLE_RESAMPLES * (MAX_DRAW_ATTEMPTS - 1)
    {
        return Err(CalyxError::assay_low_signal(
            "mixed continuous-discrete KSG subsampling plan failed its reproducibility invariants",
        ));
    }
    if ci.interval.ci_high < 0.0 {
        return Err(CalyxError::assay_low_signal(
            "mixed continuous-discrete KSG interval lies outside the non-negative MI domain",
        ));
    }
    let raw_point = raw_point_estimate as f32;
    let bits = raw_point.max(0.0);
    // A reverse-tail basic interval can exclude its generating point under finite-sample bias.
    // MiEstimate represents a point-containing uncertainty band, so take the conservative hull
    // explicitly after validating the raw root interval rather than relying on its constructor to
    // pad the bounds silently.
    let ci_low = ci.interval.ci_low.min(raw_point).max(0.0);
    let ci_high = ci.interval.ci_high.max(raw_point).max(0.0);
    if !bits.is_finite()
        || !ci_low.is_finite()
        || !ci_high.is_finite()
        || ci_low > bits
        || bits > ci_high
    {
        return Err(CalyxError::assay_low_signal(format!(
            "mixed continuous-discrete KSG interval does not contain its reported point: raw={raw_point_estimate}, bits={bits}, low={ci_low}, high={ci_high}"
        )));
    }
    Ok(
        MiEstimate::new(bits, ci_low, ci_high, n_samples, EstimatorKind::Ksg, trust)
            .with_bound(EstimateBound::Point),
    )
}
