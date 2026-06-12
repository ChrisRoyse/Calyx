//! Maximum mean discrepancy (MMD) drift tests (PRD `26 §7`, PH70).
//!
//! This module provides a deterministic Gaussian-kernel two-sample test plus a
//! simple change-point scan over an ordered feature stream. It is deliberately
//! small and fail-closed: invalid dimensions, non-finite values, too few rows,
//! or zero-signal inputs return cataloged `CALYX_ASSAY_*` errors.

use rand::SeedableRng;
use rand::seq::SliceRandom;
use rand_chacha::ChaCha8Rng;
use serde::{Deserialize, Serialize};

use calyx_core::{CalyxError, Result};

pub const MIN_MMD_SAMPLES: usize = 4;
pub const MAX_MMD_SAMPLES: usize = 2_048;
pub const DEFAULT_MMD_PERMUTATIONS: usize = 99;
pub const DEFAULT_MMD_ALPHA: f64 = 0.01;
pub const DEFAULT_MMD_SEED: u64 = 609;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct MmdConfig {
    /// Gaussian sigma. When `None`, the median pairwise distance heuristic is
    /// used over the pooled sample.
    pub bandwidth: Option<f64>,
    pub permutations: usize,
    pub seed: u64,
    pub alpha: f64,
}

impl Default for MmdConfig {
    fn default() -> Self {
        Self {
            bandwidth: None,
            permutations: DEFAULT_MMD_PERMUTATIONS,
            seed: DEFAULT_MMD_SEED,
            alpha: DEFAULT_MMD_ALPHA,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MmdReport {
    pub n_a: usize,
    pub n_b: usize,
    pub dimension: usize,
    pub bandwidth: f64,
    pub mmd2: f64,
    pub null_mean: f64,
    pub critical_value: f64,
    pub p_value: f64,
    pub significant: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChangePointReport {
    pub split_index: usize,
    pub left_n: usize,
    pub right_n: usize,
    pub report: MmdReport,
}

pub fn gaussian_mmd(x: &[Vec<f64>], y: &[Vec<f64>]) -> Result<MmdReport> {
    gaussian_mmd_with_config(x, y, &MmdConfig::default())
}

pub fn gaussian_mmd_with_config(
    x: &[Vec<f64>],
    y: &[Vec<f64>],
    config: &MmdConfig,
) -> Result<MmdReport> {
    let shape = validate_pair(x, y, config)?;
    let pooled = pooled_samples(x, y);
    let bandwidth = resolve_bandwidth(&pooled, config.bandwidth)?;
    let observed = mmd2_indexed(
        &pooled,
        &(0..x.len()).collect::<Vec<_>>(),
        &(x.len()..pooled.len()).collect::<Vec<_>>(),
        bandwidth,
    );
    let mut rng = ChaCha8Rng::seed_from_u64(config.seed);
    let mut indices: Vec<usize> = (0..pooled.len()).collect();
    let mut null = Vec::with_capacity(config.permutations);
    for _ in 0..config.permutations {
        indices.shuffle(&mut rng);
        null.push(mmd2_indexed(
            &pooled,
            &indices[..x.len()],
            &indices[x.len()..],
            bandwidth,
        ));
    }
    null.sort_by(|a, b| a.total_cmp(b));
    let ge_count = null.iter().filter(|&&sample| sample >= observed).count();
    let p_value = (ge_count + 1) as f64 / (config.permutations + 1) as f64;
    let critical_value = quantile(&null, 1.0 - config.alpha);
    let null_mean = null.iter().sum::<f64>() / null.len() as f64;
    Ok(MmdReport {
        n_a: x.len(),
        n_b: y.len(),
        dimension: shape.dimension,
        bandwidth,
        mmd2: observed,
        null_mean,
        critical_value,
        p_value,
        significant: p_value <= config.alpha && observed > critical_value,
    })
}

pub fn mmd_change_point(
    samples: &[Vec<f64>],
    min_window: usize,
    config: &MmdConfig,
) -> Result<ChangePointReport> {
    validate_single(samples, config)?;
    if min_window < MIN_MMD_SAMPLES {
        return Err(CalyxError::assay_insufficient_samples(format!(
            "MMD change-point min_window must be >= {MIN_MMD_SAMPLES}, got {min_window}"
        )));
    }
    if samples.len() < min_window * 2 {
        return Err(CalyxError::assay_insufficient_samples(format!(
            "MMD change-point requires at least {} samples, got {}",
            min_window * 2,
            samples.len()
        )));
    }
    let bandwidth = resolve_bandwidth(samples, config.bandwidth)?;
    let mut best_split = min_window;
    let mut best_mmd = f64::NEG_INFINITY;
    for split in min_window..=(samples.len() - min_window) {
        let left: Vec<usize> = (0..split).collect();
        let right: Vec<usize> = (split..samples.len()).collect();
        let value = mmd2_indexed(samples, &left, &right, bandwidth);
        if value > best_mmd {
            best_mmd = value;
            best_split = split;
        }
    }
    let mut split_config = *config;
    split_config.bandwidth = Some(bandwidth);
    let report = gaussian_mmd_with_config(
        &samples[..best_split],
        &samples[best_split..],
        &split_config,
    )?;
    Ok(ChangePointReport {
        split_index: best_split,
        left_n: best_split,
        right_n: samples.len() - best_split,
        report,
    })
}

#[derive(Clone, Copy)]
struct Shape {
    dimension: usize,
}

fn validate_pair(x: &[Vec<f64>], y: &[Vec<f64>], config: &MmdConfig) -> Result<Shape> {
    validate_config(config)?;
    if x.len() < MIN_MMD_SAMPLES || y.len() < MIN_MMD_SAMPLES {
        return Err(CalyxError::assay_insufficient_samples(format!(
            "MMD requires >= {MIN_MMD_SAMPLES} samples per side, got {} and {}",
            x.len(),
            y.len()
        )));
    }
    if x.len() + y.len() > MAX_MMD_SAMPLES {
        return Err(CalyxError::assay_insufficient_samples(format!(
            "MMD input has {} pooled samples (max {MAX_MMD_SAMPLES})",
            x.len() + y.len()
        )));
    }
    let dimension = x
        .first()
        .ok_or_else(|| CalyxError::assay_insufficient_samples("MMD side A is empty"))?
        .len();
    if dimension == 0 {
        return Err(CalyxError::assay_insufficient_samples(
            "MMD vectors must have at least one dimension",
        ));
    }
    validate_rows(x, dimension, "A")?;
    validate_rows(y, dimension, "B")?;
    Ok(Shape { dimension })
}

fn validate_single(samples: &[Vec<f64>], config: &MmdConfig) -> Result<()> {
    validate_config(config)?;
    if samples.len() < MIN_MMD_SAMPLES * 2 {
        return Err(CalyxError::assay_insufficient_samples(format!(
            "MMD change-point requires >= {} samples, got {}",
            MIN_MMD_SAMPLES * 2,
            samples.len()
        )));
    }
    if samples.len() > MAX_MMD_SAMPLES {
        return Err(CalyxError::assay_insufficient_samples(format!(
            "MMD change-point input has {} samples (max {MAX_MMD_SAMPLES})",
            samples.len()
        )));
    }
    let dimension = samples[0].len();
    if dimension == 0 {
        return Err(CalyxError::assay_insufficient_samples(
            "MMD vectors must have at least one dimension",
        ));
    }
    validate_rows(samples, dimension, "stream")
}

fn validate_config(config: &MmdConfig) -> Result<()> {
    if config.permutations == 0 {
        return Err(CalyxError::assay_insufficient_samples(
            "MMD permutations must be >= 1",
        ));
    }
    if !config.alpha.is_finite() || config.alpha <= 0.0 || config.alpha >= 1.0 {
        return Err(CalyxError::assay_insufficient_samples(format!(
            "MMD alpha must be in (0, 1), got {}",
            config.alpha
        )));
    }
    if let Some(bandwidth) = config.bandwidth
        && (!bandwidth.is_finite() || bandwidth <= 0.0)
    {
        return Err(CalyxError::assay_insufficient_samples(format!(
            "MMD bandwidth must be finite and positive, got {bandwidth}"
        )));
    }
    Ok(())
}

fn validate_rows(rows: &[Vec<f64>], dimension: usize, side: &str) -> Result<()> {
    for (row_index, row) in rows.iter().enumerate() {
        if row.len() != dimension {
            return Err(CalyxError::assay_insufficient_samples(format!(
                "MMD side {side} row {row_index} has dimension {}, expected {dimension}",
                row.len()
            )));
        }
        for (col_index, value) in row.iter().enumerate() {
            if !value.is_finite() {
                return Err(CalyxError::assay_insufficient_samples(format!(
                    "MMD side {side} row {row_index} col {col_index} is NaN or infinity"
                )));
            }
        }
    }
    Ok(())
}

fn pooled_samples(x: &[Vec<f64>], y: &[Vec<f64>]) -> Vec<Vec<f64>> {
    x.iter().chain(y.iter()).cloned().collect()
}

fn resolve_bandwidth(samples: &[Vec<f64>], configured: Option<f64>) -> Result<f64> {
    if let Some(bandwidth) = configured {
        return Ok(bandwidth);
    }
    let mut distances = Vec::new();
    for i in 0..samples.len() {
        for j in (i + 1)..samples.len() {
            let distance = squared_distance(&samples[i], &samples[j]).sqrt();
            if distance > 0.0 {
                distances.push(distance);
            }
        }
    }
    if distances.is_empty() {
        return Err(CalyxError::assay_low_signal(
            "MMD pooled samples have zero pairwise distance; no distribution shift is measurable",
        ));
    }
    distances.sort_by(|a, b| a.total_cmp(b));
    Ok(quantile(&distances, 0.5))
}

fn mmd2_indexed(samples: &[Vec<f64>], x: &[usize], y: &[usize], bandwidth: f64) -> f64 {
    mean_kernel(samples, x, x, bandwidth) + mean_kernel(samples, y, y, bandwidth)
        - 2.0 * mean_kernel(samples, x, y, bandwidth)
}

fn mean_kernel(samples: &[Vec<f64>], left: &[usize], right: &[usize], bandwidth: f64) -> f64 {
    let mut sum = 0.0;
    for &i in left {
        for &j in right {
            sum += gaussian_kernel(&samples[i], &samples[j], bandwidth);
        }
    }
    sum / (left.len() * right.len()) as f64
}

fn gaussian_kernel(a: &[f64], b: &[f64], bandwidth: f64) -> f64 {
    (-squared_distance(a, b) / (2.0 * bandwidth * bandwidth)).exp()
}

fn squared_distance(a: &[f64], b: &[f64]) -> f64 {
    a.iter()
        .zip(b.iter())
        .map(|(x, y)| {
            let delta = x - y;
            delta * delta
        })
        .sum()
}

fn quantile(sorted_values: &[f64], q: f64) -> f64 {
    debug_assert!(!sorted_values.is_empty());
    let rank = ((sorted_values.len() - 1) as f64 * q).ceil() as usize;
    sorted_values[rank.min(sorted_values.len() - 1)]
}
