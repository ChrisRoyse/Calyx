use std::collections::HashMap;

use calyx_core::{Clock, Ts};
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;

use super::{AutotuneCache, AutotuneKey, BenchResult};
use crate::BestConfig;

pub const EPSILON: f64 = 0.1;
pub const MIN_PROMOTE_MARGIN: f64 = 0.02;
pub const MIN_PROMOTE_TRIALS: u32 = 3;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExplorerPolicy {
    EpsilonGreedy,
    Thompson,
}

#[derive(Clone, Debug)]
pub struct Explorer {
    policy: ExplorerPolicy,
    rng: ChaCha8Rng,
    candidate_stats: HashMap<AutotuneKey, Vec<BenchResult>>,
    candidate_configs: HashMap<AutotuneKey, Vec<BestConfig>>,
    last_promotion_ts: Option<Ts>,
}

impl Explorer {
    pub fn new(policy: ExplorerPolicy, seed: u64) -> Self {
        Self {
            policy,
            rng: ChaCha8Rng::seed_from_u64(seed),
            candidate_stats: HashMap::new(),
            candidate_configs: HashMap::new(),
            last_promotion_ts: None,
        }
    }

    pub fn policy(&self) -> ExplorerPolicy {
        self.policy
    }

    pub fn trial_count(&self, key: &AutotuneKey) -> usize {
        self.candidate_stats.get(key).map_or(0, Vec::len)
    }

    pub fn last_promotion_ts(&self) -> Option<Ts> {
        self.last_promotion_ts
    }
}

pub fn next_candidate(
    explorer: &mut Explorer,
    key: &AutotuneKey,
    incumbent: &BestConfig,
    candidate_pool: &[BestConfig],
) -> BestConfig {
    if candidate_pool.is_empty() {
        return incumbent.clone();
    }
    match explorer.policy {
        ExplorerPolicy::EpsilonGreedy => next_epsilon_greedy(explorer, incumbent, candidate_pool),
        ExplorerPolicy::Thompson => next_thompson(explorer, key, candidate_pool),
    }
}

pub fn record_trial(
    explorer: &mut Explorer,
    key: &AutotuneKey,
    config: &BestConfig,
    result: BenchResult,
) {
    explorer
        .candidate_stats
        .entry(key.clone())
        .or_default()
        .push(result);
    explorer
        .candidate_configs
        .entry(key.clone())
        .or_default()
        .push(config.clone());
}

pub fn should_promote(
    explorer: &Explorer,
    key: &AutotuneKey,
    challenger: &BestConfig,
    incumbent: &BestConfig,
) -> bool {
    let challenger = results_for(explorer, key, challenger);
    let incumbent = results_for(explorer, key, incumbent);
    if challenger.len() < MIN_PROMOTE_TRIALS as usize
        || incumbent.len() < MIN_PROMOTE_TRIALS as usize
    {
        return false;
    }
    let challenger_mean = mean_gflops(&challenger);
    let incumbent_mean = mean_gflops(&incumbent);
    challenger_mean > incumbent_mean * (1.0 + MIN_PROMOTE_MARGIN)
}

pub fn promote_if_winner(
    explorer: &mut Explorer,
    cache: &mut AutotuneCache,
    key: AutotuneKey,
    challenger: BestConfig,
    incumbent: BestConfig,
    clock: &dyn Clock,
) -> Option<BestConfig> {
    if !should_promote(explorer, &key, &challenger, &incumbent) {
        return None;
    }
    let ts = clock.now();
    cache.insert(key, challenger);
    explorer.last_promotion_ts = Some(ts);
    Some(incumbent)
}

fn next_epsilon_greedy(
    explorer: &mut Explorer,
    incumbent: &BestConfig,
    candidate_pool: &[BestConfig],
) -> BestConfig {
    if explorer.rng.gen_range(0.0..1.0) < EPSILON {
        let idx = explorer.rng.gen_range(0..candidate_pool.len());
        candidate_pool[idx].clone()
    } else {
        incumbent.clone()
    }
}

fn next_thompson(
    explorer: &mut Explorer,
    key: &AutotuneKey,
    candidate_pool: &[BestConfig],
) -> BestConfig {
    let mut best_idx = 0;
    let mut best_score = f64::NEG_INFINITY;
    for (idx, candidate) in candidate_pool.iter().enumerate() {
        let (wins, losses) = thompson_counts(explorer, key, candidate);
        let score = sample_beta_integer(wins + 1, losses + 1, &mut explorer.rng);
        if score > best_score {
            best_score = score;
            best_idx = idx;
        }
    }
    candidate_pool[best_idx].clone()
}

fn thompson_counts(explorer: &Explorer, key: &AutotuneKey, candidate: &BestConfig) -> (u32, u32) {
    let candidate_results = results_for(explorer, key, candidate);
    if candidate_results.is_empty() {
        return (0, 0);
    }
    let all_results = explorer
        .candidate_stats
        .get(key)
        .map_or(&[][..], Vec::as_slice);
    let global_mean = mean_gflops(all_results);
    let mut wins = 0;
    let mut losses = 0;
    for result in candidate_results {
        if result.gflops >= global_mean {
            wins += 1;
        } else {
            losses += 1;
        }
    }
    (wins, losses)
}

fn results_for(explorer: &Explorer, key: &AutotuneKey, config: &BestConfig) -> Vec<BenchResult> {
    let Some(results) = explorer.candidate_stats.get(key) else {
        return Vec::new();
    };
    let Some(configs) = explorer.candidate_configs.get(key) else {
        return Vec::new();
    };
    results
        .iter()
        .zip(configs.iter())
        .filter_map(|(result, stored_config)| {
            if stored_config == config {
                Some(*result)
            } else {
                None
            }
        })
        .collect()
}

fn mean_gflops(results: &[BenchResult]) -> f64 {
    if results.is_empty() {
        return 0.0;
    }
    results.iter().map(|result| result.gflops).sum::<f64>() / results.len() as f64
}

fn sample_beta_integer(alpha: u32, beta: u32, rng: &mut ChaCha8Rng) -> f64 {
    let left = sample_gamma_integer(alpha, rng);
    let right = sample_gamma_integer(beta, rng);
    left / (left + right)
}

fn sample_gamma_integer(shape: u32, rng: &mut ChaCha8Rng) -> f64 {
    (0..shape)
        .map(|_| {
            let uniform = rng.gen_range(f64::MIN_POSITIVE..1.0);
            -uniform.ln()
        })
        .sum()
}
