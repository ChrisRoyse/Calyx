//! Transfer entropy over recurrence streams (PRD `26 §4`, PH52).
//!
//! `T(A -> B) = I(B_future; A_past, B_past) - I(B_future; B_past)`.
//! This module keeps the estimator honest by reusing the Assay KSG MI path and
//! returning provisional, code-tagged readbacks below quorum instead of
//! silently treating underpowered streams as causal evidence.

use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use serde::{Deserialize, Serialize};

use calyx_core::{CalyxError, Clock, Result, Ts};

use crate::estimate::TrustTag;
use crate::ksg::{MIN_ASSAY_SAMPLES, ksg_mi_continuous_point};

pub type Timestamp = Ts;
pub type RecurrenceStream = [(Timestamp, f32)];

pub const CALYX_TE_INSUFFICIENT_SAMPLES: &str = "CALYX_TE_INSUFFICIENT_SAMPLES";
pub const MIN_TE_QUORUM: usize = 30;
pub const DEFAULT_TE_WINDOW: usize = 1;
pub const DEFAULT_TE_K: usize = 3;
pub const DEFAULT_TE_BOOTSTRAP_RESAMPLES: usize = 500;
pub const DEFAULT_TE_BOOTSTRAP_SEED: u64 = 52;
pub const DEFAULT_TE_LAGS: &[usize] = &[1, 2, 4, 8];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Direction {
    #[serde(rename = "A_to_B")]
    AToB,
    #[serde(rename = "B_to_A")]
    BToA,
    Unclear,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TEResult {
    pub t_a_to_b: f32,
    pub t_b_to_a: f32,
    pub dominant_direction: Direction,
    pub ci_95: (f32, f32),
    pub t_b_to_a_ci_95: (f32, f32),
    pub difference_ci_95: (f32, f32),
    pub lag: usize,
    pub window_size: usize,
    pub provisional: bool,
    pub n_samples: usize,
    pub error_code: Option<String>,
    pub trust: TrustTag,
    pub computed_at: Ts,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TransferEntropyConfig {
    pub window_size: usize,
    pub k: usize,
    pub bootstrap_resamples: usize,
    pub bootstrap_seed: u64,
}

impl Default for TransferEntropyConfig {
    fn default() -> Self {
        Self {
            window_size: DEFAULT_TE_WINDOW,
            k: DEFAULT_TE_K,
            bootstrap_resamples: DEFAULT_TE_BOOTSTRAP_RESAMPLES,
            bootstrap_seed: DEFAULT_TE_BOOTSTRAP_SEED,
        }
    }
}

pub fn transfer_entropy(
    stream_a: &RecurrenceStream,
    stream_b: &RecurrenceStream,
    lag: usize,
    clock: &dyn Clock,
) -> Result<TEResult> {
    transfer_entropy_with_config(
        stream_a,
        stream_b,
        lag,
        clock,
        &TransferEntropyConfig::default(),
    )
}

pub fn transfer_entropy_with_config(
    stream_a: &RecurrenceStream,
    stream_b: &RecurrenceStream,
    lag: usize,
    clock: &dyn Clock,
    config: &TransferEntropyConfig,
) -> Result<TEResult> {
    validate_config(config)?;
    let forward = lagged_samples(stream_a, stream_b, lag, config.window_size)?;
    let reverse = lagged_samples(stream_b, stream_a, lag, config.window_size)?;
    let n_samples = forward.len().min(reverse.len());
    if n_samples < MIN_TE_QUORUM || n_samples < MIN_ASSAY_SAMPLES {
        return Ok(provisional_result(
            lag,
            config.window_size,
            n_samples,
            clock,
        ));
    }

    let forward = &forward[..n_samples];
    let reverse = &reverse[..n_samples];
    let t_a_to_b = estimate_te(forward, config.k)?;
    let t_b_to_a = estimate_te(reverse, config.k)?;
    let ci_95 = bootstrap_ci(forward, t_a_to_b, config, config.bootstrap_seed)?;
    let reverse_ci_95 = bootstrap_ci(
        reverse,
        t_b_to_a,
        config,
        config.bootstrap_seed ^ 0x0B17_B1D5,
    )?;
    let difference_ci_95 = bootstrap_difference_ci(
        forward,
        reverse,
        t_a_to_b - t_b_to_a,
        config,
        config.bootstrap_seed ^ 0x00D1_FFC1,
    )?;
    Ok(TEResult {
        t_a_to_b,
        t_b_to_a,
        dominant_direction: dominant_direction(t_a_to_b, t_b_to_a, ci_95, reverse_ci_95),
        ci_95,
        t_b_to_a_ci_95: reverse_ci_95,
        difference_ci_95,
        lag,
        window_size: config.window_size,
        provisional: false,
        n_samples,
        error_code: None,
        trust: TrustTag::Provisional,
        computed_at: clock.now(),
    })
}

pub fn transfer_entropy_sweep(
    a: &RecurrenceStream,
    b: &RecurrenceStream,
    lags: &[usize],
    clock: &dyn Clock,
) -> Vec<TEResult> {
    transfer_entropy_sweep_with_config(a, b, lags, clock, &TransferEntropyConfig::default())
}

pub fn transfer_entropy_sweep_with_config(
    a: &RecurrenceStream,
    b: &RecurrenceStream,
    lags: &[usize],
    clock: &dyn Clock,
    config: &TransferEntropyConfig,
) -> Vec<TEResult> {
    lags.iter()
        .map(
            |&lag| match transfer_entropy_with_config(a, b, lag, clock, config) {
                Ok(result) => result,
                Err(error) => error_result(lag, config.window_size, clock, error),
            },
        )
        .collect()
}

pub fn max_transfer_entropy_lag(results: &[TEResult]) -> Option<usize> {
    results
        .iter()
        .filter(|result| !result.provisional)
        .max_by(|left, right| left.t_a_to_b.total_cmp(&right.t_a_to_b))
        .map(|result| result.lag)
}

fn validate_config(config: &TransferEntropyConfig) -> Result<()> {
    if config.window_size == 0 || config.k == 0 || config.bootstrap_resamples == 0 {
        return Err(insufficient(
            "transfer entropy requires window_size > 0, k > 0, and bootstrap_resamples > 0",
        ));
    }
    Ok(())
}

fn provisional_result(
    lag: usize,
    window_size: usize,
    n_samples: usize,
    clock: &dyn Clock,
) -> TEResult {
    TEResult {
        t_a_to_b: 0.0,
        t_b_to_a: 0.0,
        dominant_direction: Direction::Unclear,
        ci_95: (0.0, 0.0),
        t_b_to_a_ci_95: (0.0, 0.0),
        difference_ci_95: (0.0, 0.0),
        lag,
        window_size,
        provisional: true,
        n_samples,
        error_code: Some(CALYX_TE_INSUFFICIENT_SAMPLES.to_string()),
        trust: TrustTag::Provisional,
        computed_at: clock.now(),
    }
}

fn error_result(lag: usize, window_size: usize, clock: &dyn Clock, error: CalyxError) -> TEResult {
    let mut result = provisional_result(lag, window_size, 0, clock);
    result.error_code = Some(error.code.to_string());
    result
}

#[derive(Clone, Debug)]
struct LaggedSample {
    future: Vec<f32>,
    joint_past: Vec<f32>,
    own_past: Vec<f32>,
}

fn lagged_samples(
    source: &RecurrenceStream,
    target: &RecurrenceStream,
    lag: usize,
    window_size: usize,
) -> Result<Vec<LaggedSample>> {
    let source = validated_map("source", source)?;
    let target = validated_map("target", target)?;
    let mut samples = Vec::new();
    for &time in source.keys() {
        let Some(future_time) = time.checked_add(lag as u64) else {
            continue;
        };
        let Some(&future) = target.get(&future_time) else {
            continue;
        };
        let Some(source_past) = history(&source, time, window_size) else {
            continue;
        };
        let Some(target_past) = history(&target, time, window_size) else {
            continue;
        };
        let mut joint_past = source_past.clone();
        joint_past.extend_from_slice(&target_past);
        samples.push(LaggedSample {
            future: vec![future],
            joint_past,
            own_past: target_past,
        });
    }
    Ok(samples)
}

fn validated_map(
    name: &'static str,
    stream: &RecurrenceStream,
) -> Result<std::collections::BTreeMap<Timestamp, f32>> {
    let mut map = std::collections::BTreeMap::new();
    for (index, &(time, value)) in stream.iter().enumerate() {
        if !value.is_finite() {
            return Err(insufficient(format!(
                "{name} sample {index} has non-finite value"
            )));
        }
        if map.insert(time, value).is_some() {
            return Err(insufficient(format!(
                "{name} has duplicate timestamp {time}"
            )));
        }
    }
    Ok(map)
}

fn history(
    map: &std::collections::BTreeMap<Timestamp, f32>,
    time: Timestamp,
    window_size: usize,
) -> Option<Vec<f32>> {
    let start = time.checked_sub(window_size.saturating_sub(1) as u64)?;
    let mut values = Vec::with_capacity(window_size);
    for t in start..=time {
        values.push(*map.get(&t)?);
    }
    Some(values)
}

fn estimate_te(samples: &[LaggedSample], k: usize) -> Result<f32> {
    let future: Vec<_> = samples.iter().map(|sample| sample.future.clone()).collect();
    let joint_past: Vec<_> = samples
        .iter()
        .map(|sample| sample.joint_past.clone())
        .collect();
    let own_past: Vec<_> = samples
        .iter()
        .map(|sample| sample.own_past.clone())
        .collect();
    let joint = ksg_mi_continuous_point(&future, &joint_past, k)?;
    let own = ksg_mi_continuous_point(&future, &own_past, k)?;
    Ok((joint - own).max(0.0))
}

fn bootstrap_ci(
    samples: &[LaggedSample],
    point: f32,
    config: &TransferEntropyConfig,
    seed: u64,
) -> Result<(f32, f32)> {
    let mut rng = ChaCha8Rng::seed_from_u64(seed);
    let mut estimates = Vec::with_capacity(config.bootstrap_resamples);
    for _ in 0..config.bootstrap_resamples {
        let resampled = resample(samples, &mut rng);
        estimates.push(estimate_te(&resampled, config.k)?);
    }
    Ok(percentile_ci(estimates, point))
}

fn bootstrap_difference_ci(
    forward: &[LaggedSample],
    reverse: &[LaggedSample],
    point: f32,
    config: &TransferEntropyConfig,
    seed: u64,
) -> Result<(f32, f32)> {
    let mut rng = ChaCha8Rng::seed_from_u64(seed);
    let mut estimates = Vec::with_capacity(config.bootstrap_resamples);
    for _ in 0..config.bootstrap_resamples {
        let f = resample(forward, &mut rng);
        let r = resample(reverse, &mut rng);
        estimates.push(estimate_te(&f, config.k)? - estimate_te(&r, config.k)?);
    }
    Ok(percentile_ci(estimates, point))
}

fn resample(samples: &[LaggedSample], rng: &mut ChaCha8Rng) -> Vec<LaggedSample> {
    (0..samples.len())
        .map(|_| samples[rng.gen_range(0..samples.len())].clone())
        .collect()
}

fn percentile_ci(mut estimates: Vec<f32>, point: f32) -> (f32, f32) {
    estimates.sort_by(f32::total_cmp);
    let low = estimates[percentile_index(estimates.len(), 0.025)].min(point);
    let high = estimates[percentile_index(estimates.len(), 0.975)].max(point);
    (low, high)
}

fn percentile_index(len: usize, p: f32) -> usize {
    let last = len.saturating_sub(1);
    ((last as f32 * p).round() as usize).min(last)
}

fn dominant_direction(
    forward: f32,
    reverse: f32,
    forward_ci: (f32, f32),
    reverse_ci: (f32, f32),
) -> Direction {
    if forward > reverse && forward_ci.0 > reverse_ci.1 {
        Direction::AToB
    } else if reverse > forward && reverse_ci.0 > forward_ci.1 {
        Direction::BToA
    } else {
        Direction::Unclear
    }
}

fn insufficient(message: impl Into<String>) -> CalyxError {
    CalyxError::assay_insufficient_samples(message)
}
