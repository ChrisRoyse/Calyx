//! Total correlation and interaction information for Assay panels.
//!
//! Total correlation is the multivariate mutual information
//! `TC(Phi) = sum_k H(slot_k) - H(Phi)`. It complements the cheap pairwise
//! differentiation gate; it does not replace that first-pass admission filter.

use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use serde::{Deserialize, Serialize};

use calyx_core::{CalyxError, Clock, Result, Ts};

use crate::estimate::TrustTag;
use crate::ksg::{MIN_ASSAY_SAMPLES, ksg_mi_continuous_point};
use crate::samples::validate_rectangular_finite;

pub type SlotVectors = [Vec<f32>];

pub const CALYX_TC_INSUFFICIENT_SAMPLES: &str = "CALYX_TC_INSUFFICIENT_SAMPLES";
pub const MIN_QUORUM_TC_PER_SLOT: usize = 50;
pub const DEFAULT_TC_K: usize = 3;
pub const DEFAULT_TC_BOOTSTRAP_RESAMPLES: usize = 500;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TotalCorrelationConfig {
    pub k: usize,
    pub bootstrap_resamples: usize,
}

impl Default for TotalCorrelationConfig {
    fn default() -> Self {
        Self {
            k: DEFAULT_TC_K,
            bootstrap_resamples: DEFAULT_TC_BOOTSTRAP_RESAMPLES,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum IISign {
    Redundant,
    Synergistic,
    Unclear,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TCResult {
    pub tc: f32,
    pub n_eff: f32,
    pub ci_95: (f32, f32),
    pub n_samples: usize,
    pub slot_count: usize,
    pub sum_marginal_entropy: f32,
    pub joint_entropy: f32,
    pub provisional: bool,
    pub error_code: Option<String>,
    pub trust: TrustTag,
    pub computed_at: Ts,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct IIResult {
    pub ii: f32,
    pub sign: IISign,
    pub ci_95: (f32, f32),
    pub n_samples: usize,
    pub provisional: bool,
    pub error_code: Option<String>,
    pub trust: TrustTag,
    pub computed_at: Ts,
}

pub fn total_correlation(slots: &SlotVectors, clock: &dyn Clock) -> Result<TCResult> {
    total_correlation_with_config(slots, clock, &TotalCorrelationConfig::default())
}

pub fn min_quorum_tc(slot_count: usize) -> usize {
    MIN_QUORUM_TC_PER_SLOT.saturating_mul(slot_count)
}

pub fn total_correlation_with_config(
    slots: &SlotVectors,
    clock: &dyn Clock,
    config: &TotalCorrelationConfig,
) -> Result<TCResult> {
    validate_config(config)?;
    let n_samples = validate_panel(slots)?;
    let slot_count = slots.len();
    if below_tc_quorum(n_samples, slot_count) {
        return Ok(provisional_tc(slot_count, n_samples, clock));
    }

    let estimate = estimate_total_correlation(slots, config.k)?;
    let ci_95 = if slot_count <= 1 {
        (0.0, 0.0)
    } else {
        bootstrap_tc_ci(slots, estimate.tc, config, clock.now() ^ 0x7C52_0001)?
    };
    Ok(TCResult {
        tc: estimate.tc,
        n_eff: estimate.n_eff,
        ci_95,
        n_samples,
        slot_count,
        sum_marginal_entropy: estimate.sum_marginal_entropy,
        joint_entropy: estimate.joint_entropy,
        provisional: false,
        error_code: None,
        trust: TrustTag::Provisional,
        computed_at: clock.now(),
    })
}

pub fn n_eff_from_tc(slot_count: usize, tc: f32, sum_marginal_entropy: f32) -> f32 {
    match slot_count {
        0 => 0.0,
        1 => 1.0,
        n => {
            let denominator = if sum_marginal_entropy > f32::EPSILON {
                sum_marginal_entropy
            } else {
                tc.max(f32::EPSILON)
            };
            let raw = n as f32 * (1.0 - tc.max(0.0) / denominator);
            raw.clamp(1.0, n as f32)
        }
    }
}

pub fn interaction_information(
    slot_a: &[f32],
    slot_b: &[f32],
    slot_c: &[f32],
    clock: &dyn Clock,
) -> Result<IIResult> {
    interaction_information_with_config(
        slot_a,
        slot_b,
        slot_c,
        clock,
        &TotalCorrelationConfig::default(),
    )
}

pub fn interaction_information_with_config(
    slot_a: &[f32],
    slot_b: &[f32],
    slot_c: &[f32],
    clock: &dyn Clock,
    config: &TotalCorrelationConfig,
) -> Result<IIResult> {
    validate_config(config)?;
    let n_samples = validate_triple(slot_a, slot_b, slot_c)?;
    if n_samples < MIN_QUORUM_TC_PER_SLOT * 3 || n_samples < MIN_ASSAY_SAMPLES {
        return Ok(provisional_ii(n_samples, clock));
    }

    let point = estimate_interaction_information(slot_a, slot_b, slot_c, config.k)?;
    let ci_95 = bootstrap_ii_ci(
        slot_a,
        slot_b,
        slot_c,
        point,
        config,
        clock.now() ^ 0x7C52_0002,
    )?;
    Ok(IIResult {
        ii: point,
        sign: ii_sign(ci_95),
        ci_95,
        n_samples,
        provisional: false,
        error_code: None,
        trust: TrustTag::Provisional,
        computed_at: clock.now(),
    })
}

#[derive(Clone, Copy, Debug)]
struct TCEstimate {
    tc: f32,
    n_eff: f32,
    sum_marginal_entropy: f32,
    joint_entropy: f32,
}

fn estimate_total_correlation(slots: &SlotVectors, k: usize) -> Result<TCEstimate> {
    if slots.len() <= 1 {
        let joint_entropy = slots
            .first()
            .map(|slot| entropy_bits_ksg(&one_dim(slot), k))
            .transpose()?
            .unwrap_or(0.0);
        return Ok(TCEstimate {
            tc: 0.0,
            n_eff: slots.len() as f32,
            sum_marginal_entropy: joint_entropy,
            joint_entropy,
        });
    }
    let mut sum_marginal_entropy = 0.0;
    for slot in slots {
        sum_marginal_entropy += entropy_bits_ksg(&one_dim(slot), k)?;
    }
    let joint = joint_matrix(slots);
    let joint_entropy = entropy_bits_ksg(&joint, k)?;
    let tc = (sum_marginal_entropy - joint_entropy).max(0.0);
    Ok(TCEstimate {
        tc,
        n_eff: n_eff_from_tc(slots.len(), tc, sum_marginal_entropy),
        sum_marginal_entropy,
        joint_entropy,
    })
}

fn estimate_interaction_information(a: &[f32], b: &[f32], c: &[f32], k: usize) -> Result<f32> {
    let a = one_dim(a);
    let b = one_dim(b);
    let c = one_dim(c);
    let bc: Vec<_> = b
        .iter()
        .zip(&c)
        .map(|(left, right)| vec![left[0], right[0]])
        .collect();
    let i_ab = ksg_mi_continuous_point(&a, &b, k)?;
    let i_a_bc = ksg_mi_continuous_point(&a, &bc, k)?;
    let i_ac = ksg_mi_continuous_point(&a, &c, k)?;
    let conditional = (i_a_bc - i_ac).max(0.0);
    Ok(i_ab - conditional)
}

fn entropy_bits_ksg(samples: &[Vec<f32>], k: usize) -> Result<f32> {
    let dim = validate_rectangular_finite("entropy samples", samples)?;
    let n = samples.len();
    if n < MIN_ASSAY_SAMPLES || k == 0 || k >= n {
        return Err(insufficient(format!(
            "KSG entropy requires at least {MIN_ASSAY_SAMPLES} samples and 0 < k < n; got n={n}, k={k}"
        )));
    }
    let mean_log_radius = (0..n)
        .map(|i| kth_radius(samples, i, k).max(f32::EPSILON).ln() as f64)
        .sum::<f64>()
        / n as f64;
    let dim = dim as f64;
    let h_nats =
        digamma(n as f64) - digamma(k as f64) + dim * (std::f64::consts::LN_2 + mean_log_radius);
    Ok((h_nats / std::f64::consts::LN_2) as f32)
}

fn bootstrap_tc_ci(
    slots: &SlotVectors,
    point: f32,
    config: &TotalCorrelationConfig,
    seed: u64,
) -> Result<(f32, f32)> {
    let mut rng = ChaCha8Rng::seed_from_u64(seed);
    let mut estimates = Vec::with_capacity(config.bootstrap_resamples);
    for _ in 0..config.bootstrap_resamples {
        let resampled = resample_slots(slots, &mut rng);
        estimates.push(estimate_total_correlation(&resampled, config.k)?.tc);
    }
    Ok(percentile_ci(estimates, point))
}

fn bootstrap_ii_ci(
    a: &[f32],
    b: &[f32],
    c: &[f32],
    point: f32,
    config: &TotalCorrelationConfig,
    seed: u64,
) -> Result<(f32, f32)> {
    let mut rng = ChaCha8Rng::seed_from_u64(seed);
    let mut estimates = Vec::with_capacity(config.bootstrap_resamples);
    for _ in 0..config.bootstrap_resamples {
        let (ra, rb, rc) = resample_triple(a, b, c, &mut rng);
        estimates.push(estimate_interaction_information(&ra, &rb, &rc, config.k)?);
    }
    Ok(percentile_ci(estimates, point))
}

fn validate_panel(slots: &SlotVectors) -> Result<usize> {
    let Some(first) = slots.first() else {
        return Ok(0);
    };
    let n = first.len();
    for (slot_index, slot) in slots.iter().enumerate() {
        if slot.len() != n {
            return Err(insufficient(format!(
                "slot {slot_index} has {} samples, expected {n}",
                slot.len()
            )));
        }
        if slot.iter().any(|value| !value.is_finite()) {
            return Err(insufficient(format!(
                "slot {slot_index} contains NaN or infinity"
            )));
        }
    }
    Ok(n)
}

fn validate_triple(a: &[f32], b: &[f32], c: &[f32]) -> Result<usize> {
    if a.len() != b.len() || a.len() != c.len() {
        return Err(insufficient(format!(
            "interaction information requires equal sample counts; got a={}, b={}, c={}",
            a.len(),
            b.len(),
            c.len()
        )));
    }
    for (name, slot) in [("a", a), ("b", b), ("c", c)] {
        if slot.iter().any(|value| !value.is_finite()) {
            return Err(insufficient(format!(
                "slot {name} contains NaN or infinity"
            )));
        }
    }
    Ok(a.len())
}

fn below_tc_quorum(n_samples: usize, slot_count: usize) -> bool {
    slot_count == 0 || n_samples < MIN_ASSAY_SAMPLES || n_samples < min_quorum_tc(slot_count)
}

fn validate_config(config: &TotalCorrelationConfig) -> Result<()> {
    if config.k == 0 || config.bootstrap_resamples == 0 {
        return Err(insufficient(
            "total correlation requires k > 0 and bootstrap_resamples > 0",
        ));
    }
    Ok(())
}

fn provisional_tc(slot_count: usize, n_samples: usize, clock: &dyn Clock) -> TCResult {
    TCResult {
        tc: 0.0,
        n_eff: slot_count as f32,
        ci_95: (0.0, 0.0),
        n_samples,
        slot_count,
        sum_marginal_entropy: 0.0,
        joint_entropy: 0.0,
        provisional: true,
        error_code: Some(CALYX_TC_INSUFFICIENT_SAMPLES.to_string()),
        trust: TrustTag::Provisional,
        computed_at: clock.now(),
    }
}

fn provisional_ii(n_samples: usize, clock: &dyn Clock) -> IIResult {
    IIResult {
        ii: 0.0,
        sign: IISign::Unclear,
        ci_95: (0.0, 0.0),
        n_samples,
        provisional: true,
        error_code: Some(CALYX_TC_INSUFFICIENT_SAMPLES.to_string()),
        trust: TrustTag::Provisional,
        computed_at: clock.now(),
    }
}

fn one_dim(slot: &[f32]) -> Vec<Vec<f32>> {
    slot.iter().map(|&value| vec![value]).collect()
}

fn joint_matrix(slots: &SlotVectors) -> Vec<Vec<f32>> {
    let n = slots.first().map_or(0, Vec::len);
    (0..n)
        .map(|sample| slots.iter().map(|slot| slot[sample]).collect())
        .collect()
}

fn resample_slots(slots: &SlotVectors, rng: &mut ChaCha8Rng) -> Vec<Vec<f32>> {
    let n = slots[0].len();
    let mut resampled = vec![Vec::with_capacity(n); slots.len()];
    for _ in 0..n {
        let index = rng.gen_range(0..n);
        for (slot_index, slot) in slots.iter().enumerate() {
            resampled[slot_index].push(slot[index]);
        }
    }
    resampled
}

fn resample_triple(
    a: &[f32],
    b: &[f32],
    c: &[f32],
    rng: &mut ChaCha8Rng,
) -> (Vec<f32>, Vec<f32>, Vec<f32>) {
    let mut ra = Vec::with_capacity(a.len());
    let mut rb = Vec::with_capacity(a.len());
    let mut rc = Vec::with_capacity(a.len());
    for _ in 0..a.len() {
        let index = rng.gen_range(0..a.len());
        ra.push(a[index]);
        rb.push(b[index]);
        rc.push(c[index]);
    }
    (ra, rb, rc)
}

fn kth_radius(samples: &[Vec<f32>], i: usize, k: usize) -> f32 {
    let mut distances = Vec::with_capacity(samples.len().saturating_sub(1));
    for j in 0..samples.len() {
        if i != j {
            distances.push(chebyshev(&samples[i], &samples[j]));
        }
    }
    distances.sort_by(f32::total_cmp);
    distances[k - 1]
}

fn chebyshev(a: &[f32], b: &[f32]) -> f32 {
    a.iter()
        .zip(b)
        .map(|(left, right)| (left - right).abs())
        .fold(0.0, f32::max)
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

fn ii_sign(ci: (f32, f32)) -> IISign {
    if ci.0 > 0.0 {
        IISign::Redundant
    } else if ci.1 < 0.0 {
        IISign::Synergistic
    } else {
        IISign::Unclear
    }
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

fn insufficient(message: impl Into<String>) -> CalyxError {
    CalyxError::assay_insufficient_samples(message)
}
