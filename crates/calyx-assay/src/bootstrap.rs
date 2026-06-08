//! Deterministic bootstrap confidence intervals.

use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use serde::{Deserialize, Serialize};

pub const DEFAULT_BOOTSTRAP_RESAMPLES: usize = 200;
pub const DEFAULT_BOOTSTRAP_SEED: u64 = 0;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BootstrapConfig {
    pub resamples: usize,
    pub seed: u64,
}

impl BootstrapConfig {
    pub const fn new(resamples: usize, seed: u64) -> Self {
        Self { resamples, seed }
    }
}

impl Default for BootstrapConfig {
    fn default() -> Self {
        Self {
            resamples: DEFAULT_BOOTSTRAP_RESAMPLES,
            seed: DEFAULT_BOOTSTRAP_SEED,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BootstrapCi {
    pub mean: f32,
    pub ci_low: f32,
    pub ci_high: f32,
    pub resamples: usize,
}

pub fn bootstrap_mean_ci(values: &[f32], resamples: usize, seed: u64) -> Option<BootstrapCi> {
    bootstrap_mean_ci_with_config(values, BootstrapConfig::new(resamples, seed))
}

pub fn bootstrap_mean_ci_with_config(
    values: &[f32],
    config: BootstrapConfig,
) -> Option<BootstrapCi> {
    if values.is_empty() || config.resamples == 0 {
        return None;
    }
    let mut rng = ChaCha8Rng::seed_from_u64(config.seed);
    let mut means = Vec::with_capacity(config.resamples);
    for _ in 0..config.resamples {
        let mut sum = 0.0;
        for _ in 0..values.len() {
            sum += values[rng.gen_range(0..values.len())];
        }
        means.push(sum / values.len() as f32);
    }
    Some(ci_from_estimates(
        means,
        values.iter().sum::<f32>() / values.len() as f32,
    ))
}

pub fn bootstrap_paired_ci<T, U, E, F>(
    left: &[T],
    right: &[U],
    point_estimate: f32,
    config: BootstrapConfig,
    mut estimator: F,
) -> std::result::Result<Option<BootstrapCi>, E>
where
    T: Clone,
    U: Clone,
    F: FnMut(&[T], &[U]) -> std::result::Result<f32, E>,
{
    if left.is_empty() || left.len() != right.len() || config.resamples == 0 {
        return Ok(None);
    }
    let mut rng = ChaCha8Rng::seed_from_u64(config.seed);
    let mut estimates = Vec::with_capacity(config.resamples);
    for _ in 0..config.resamples {
        let mut sampled_left = Vec::with_capacity(left.len());
        let mut sampled_right = Vec::with_capacity(right.len());
        for _ in 0..left.len() {
            let index = rng.gen_range(0..left.len());
            sampled_left.push(left[index].clone());
            sampled_right.push(right[index].clone());
        }
        estimates.push(estimator(&sampled_left, &sampled_right)?);
    }
    Ok(Some(ci_from_estimates(estimates, point_estimate)))
}

fn ci_from_estimates(mut estimates: Vec<f32>, point_estimate: f32) -> BootstrapCi {
    estimates.sort_by(f32::total_cmp);
    let low_index = percentile_index(estimates.len(), 0.025);
    let high_index = percentile_index(estimates.len(), 0.975);
    let percentile_low = estimates[low_index];
    let percentile_high = estimates[high_index];
    let bootstrap_span = (percentile_high - percentile_low).max(0.0);
    BootstrapCi {
        mean: point_estimate,
        ci_low: (percentile_low - bootstrap_span).min(point_estimate),
        ci_high: (percentile_high + bootstrap_span).max(point_estimate),
        resamples: estimates.len(),
    }
}

fn percentile_index(len: usize, p: f32) -> usize {
    let last = len.saturating_sub(1);
    ((last as f32 * p).round() as usize).min(last)
}
