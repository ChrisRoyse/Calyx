//! Deterministic bootstrap confidence intervals.

use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BootstrapCi {
    pub mean: f32,
    pub ci_low: f32,
    pub ci_high: f32,
    pub resamples: usize,
}

pub fn bootstrap_mean_ci(values: &[f32], resamples: usize, seed: u64) -> Option<BootstrapCi> {
    if values.is_empty() || resamples == 0 {
        return None;
    }
    let mut rng = ChaCha8Rng::seed_from_u64(seed);
    let mut means = Vec::with_capacity(resamples);
    for _ in 0..resamples {
        let mut sum = 0.0;
        for _ in 0..values.len() {
            sum += values[rng.gen_range(0..values.len())];
        }
        means.push(sum / values.len() as f32);
    }
    means.sort_by(f32::total_cmp);
    let low_index = percentile_index(resamples, 0.025);
    let high_index = percentile_index(resamples, 0.975);
    Some(BootstrapCi {
        mean: values.iter().sum::<f32>() / values.len() as f32,
        ci_low: means[low_index],
        ci_high: means[high_index],
        resamples,
    })
}

fn percentile_index(len: usize, p: f32) -> usize {
    let last = len.saturating_sub(1);
    ((last as f32 * p).round() as usize).min(last)
}
