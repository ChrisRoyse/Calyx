use std::collections::HashMap;

use calyx_core::{Clock, Ts};
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;

use super::{AutotuneCache, AutotuneKey, BenchResult};
use crate::BestConfig;

pub const EPSILON: f64 = 0.1;
pub const MIN_PROMOTE_MARGIN: f64 = 0.02;
pub const MIN_PROMOTE_TRIALS: u32 = 3;
const THOMPSON_COUNT_CAP: u32 = 128;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExplorerPolicy {
    EpsilonGreedy,
    Thompson,
}

#[derive(Clone, Debug)]
pub struct Explorer {
    policy: ExplorerPolicy,
    rng: ChaCha8Rng,
    candidate_stats: HashMap<AutotuneKey, CandidateStats>,
    last_promotion_ts: Option<Ts>,
}

#[derive(Clone, Debug, Default)]
struct CandidateStats {
    total: RunningGflops,
    configs: Vec<ConfigStats>,
}

#[derive(Clone, Debug)]
struct ConfigStats {
    config: BestConfig,
    gflops: RunningGflops,
}

#[derive(Clone, Copy, Debug, Default)]
struct RunningGflops {
    count: usize,
    mean: f64,
}

impl Explorer {
    pub fn new(policy: ExplorerPolicy, seed: u64) -> Self {
        Self {
            policy,
            rng: ChaCha8Rng::seed_from_u64(seed),
            candidate_stats: HashMap::new(),
            last_promotion_ts: None,
        }
    }

    pub fn policy(&self) -> ExplorerPolicy {
        self.policy
    }

    pub fn trial_count(&self, key: &AutotuneKey) -> usize {
        self.candidate_stats
            .get(key)
            .map_or(0, CandidateStats::trial_count)
    }

    pub fn last_promotion_ts(&self) -> Option<Ts> {
        self.last_promotion_ts
    }
}

impl CandidateStats {
    fn record(&mut self, config: &BestConfig, result: BenchResult) {
        self.total.record(result);
        if let Some(entry) = self
            .configs
            .iter_mut()
            .find(|entry| entry.config == *config)
        {
            entry.gflops.record(result);
            return;
        }
        let mut gflops = RunningGflops::default();
        gflops.record(result);
        self.configs.push(ConfigStats {
            config: config.clone(),
            gflops,
        });
    }

    fn aggregate_for(&self, config: &BestConfig) -> Option<RunningGflops> {
        self.configs
            .iter()
            .find(|entry| entry.config == *config)
            .map(|entry| entry.gflops)
    }

    fn trial_count(&self) -> usize {
        self.total.count
    }
}

impl RunningGflops {
    fn record(&mut self, result: BenchResult) {
        self.count = self.count.saturating_add(1);
        self.mean += (result.gflops - self.mean) / self.count as f64;
    }

    fn thompson_count(self) -> u32 {
        self.count.min(THOMPSON_COUNT_CAP as usize) as u32
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
        ExplorerPolicy::Thompson => next_thompson(explorer, key, incumbent, candidate_pool),
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
        .record(config, result);
}

pub fn should_promote(
    explorer: &Explorer,
    key: &AutotuneKey,
    challenger: &BestConfig,
    incumbent: &BestConfig,
) -> bool {
    let Some(challenger) = aggregate_for(explorer, key, challenger) else {
        return false;
    };
    let Some(incumbent) = aggregate_for(explorer, key, incumbent) else {
        return false;
    };
    if challenger.count < MIN_PROMOTE_TRIALS as usize
        || incumbent.count < MIN_PROMOTE_TRIALS as usize
    {
        return false;
    }
    challenger.mean > incumbent.mean * (1.0 + MIN_PROMOTE_MARGIN)
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
    if explorer.rng.random_range(0.0..1.0) < EPSILON {
        let idx = explorer.rng.random_range(0..candidate_pool.len());
        candidate_pool[idx].clone()
    } else {
        incumbent.clone()
    }
}

fn next_thompson(
    explorer: &mut Explorer,
    key: &AutotuneKey,
    incumbent: &BestConfig,
    candidate_pool: &[BestConfig],
) -> BestConfig {
    let mut best_idx = 0;
    let mut best_score = f64::NEG_INFINITY;
    for (idx, candidate) in candidate_pool.iter().enumerate() {
        let (wins, losses) = thompson_counts(explorer, key, candidate, incumbent);
        let score = sample_beta_integer(
            wins.saturating_add(1),
            losses.saturating_add(1),
            &mut explorer.rng,
        );
        if score > best_score {
            best_score = score;
            best_idx = idx;
        }
    }
    candidate_pool[best_idx].clone()
}

fn thompson_counts(
    explorer: &Explorer,
    key: &AutotuneKey,
    candidate: &BestConfig,
    incumbent: &BestConfig,
) -> (u32, u32) {
    let Some(candidate) = aggregate_for(explorer, key, candidate) else {
        return (0, 0);
    };
    let Some(incumbent) = aggregate_for(explorer, key, incumbent) else {
        return (0, 0);
    };
    let trials = candidate.thompson_count();
    if trials == 0 {
        return (0, 0);
    }
    if candidate.mean > incumbent.mean * (1.0 + MIN_PROMOTE_MARGIN) {
        (trials, 0)
    } else if incumbent.mean > candidate.mean * (1.0 + MIN_PROMOTE_MARGIN) {
        (0, trials)
    } else {
        let wins = trials / 2;
        (wins, trials - wins)
    }
}

fn aggregate_for(
    explorer: &Explorer,
    key: &AutotuneKey,
    config: &BestConfig,
) -> Option<RunningGflops> {
    explorer
        .candidate_stats
        .get(key)
        .and_then(|stats| stats.aggregate_for(config))
}

fn sample_beta_integer(alpha: u32, beta: u32, rng: &mut ChaCha8Rng) -> f64 {
    let left = sample_gamma_integer(alpha, rng);
    let right = sample_gamma_integer(beta, rng);
    left / (left + right)
}

fn sample_gamma_integer(shape: u32, rng: &mut ChaCha8Rng) -> f64 {
    (0..shape)
        .map(|_| {
            let uniform = rng.random_range(f64::MIN_POSITIVE..1.0);
            -uniform.ln()
        })
        .sum()
}
