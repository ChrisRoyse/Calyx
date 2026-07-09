//! Convergent Cross Mapping for deterministic-ish scalar systems (#64).
//!
//! This is a local scalar CCM estimator: delay-coordinate embeddings, leave-one-
//! out simplex cross mapping, and directional evidence from convergence plus
//! final cross-map skill. It is not a general nonlinear causal discovery stack.

use calyx_core::{CalyxError, Result};
use serde::{Deserialize, Serialize};

pub const DEFAULT_CCM_EMBEDDING_DIM: usize = 3;
pub const DEFAULT_CCM_TAU: usize = 1;
pub const DEFAULT_CCM_MIN_CONVERGENCE_DELTA: f32 = 0.05;
pub const DEFAULT_CCM_MIN_SKILL_GAP: f32 = 0.05;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CcmConfig {
    pub embedding_dim: usize,
    pub tau: usize,
    pub library_sizes: Vec<usize>,
    pub min_convergence_delta: f32,
    pub min_skill_gap: f32,
}

impl CcmConfig {
    pub fn new(
        embedding_dim: usize,
        tau: usize,
        library_sizes: Vec<usize>,
        min_convergence_delta: f32,
        min_skill_gap: f32,
    ) -> Self {
        Self {
            embedding_dim,
            tau,
            library_sizes,
            min_convergence_delta,
            min_skill_gap,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum CcmVerdict {
    XCausesY,
    YCausesX,
    Bidirectional,
    Inconclusive,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct CcmLibrarySkill {
    pub library_size: usize,
    pub rho: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CcmDirectionReport {
    pub manifold: String,
    pub target: String,
    pub library_skills: Vec<CcmLibrarySkill>,
    pub final_rho: f32,
    pub convergence_delta: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CcmReport {
    pub estimator: String,
    pub x_name: String,
    pub y_name: String,
    pub embedding_dim: usize,
    pub tau: usize,
    pub neighbor_count: usize,
    pub n_samples: usize,
    pub effective_points: usize,
    pub min_convergence_delta: f32,
    pub min_skill_gap: f32,
    pub x_manifold_to_y: CcmDirectionReport,
    pub y_manifold_to_x: CcmDirectionReport,
    pub verdict: CcmVerdict,
}

pub fn convergent_cross_mapping(
    x_name: &str,
    x: &[f32],
    y_name: &str,
    y: &[f32],
    config: &CcmConfig,
) -> Result<CcmReport> {
    let effective_points = validate_ccm_inputs(x_name, x, y_name, y, config)?;
    let x_to_y = cross_map_direction(
        x_name,
        x,
        y_name,
        y,
        config.embedding_dim,
        config.tau,
        &config.library_sizes,
    )?;
    let y_to_x = cross_map_direction(
        y_name,
        y,
        x_name,
        x,
        config.embedding_dim,
        config.tau,
        &config.library_sizes,
    )?;
    let verdict = ccm_verdict(
        &x_to_y,
        &y_to_x,
        config.min_convergence_delta,
        config.min_skill_gap,
    );

    Ok(CcmReport {
        estimator: "convergent_cross_mapping_simplex".to_string(),
        x_name: x_name.to_string(),
        y_name: y_name.to_string(),
        embedding_dim: config.embedding_dim,
        tau: config.tau,
        neighbor_count: config.embedding_dim + 1,
        n_samples: x.len(),
        effective_points,
        min_convergence_delta: config.min_convergence_delta,
        min_skill_gap: config.min_skill_gap,
        x_manifold_to_y: x_to_y,
        y_manifold_to_x: y_to_x,
        verdict,
    })
}

fn validate_ccm_inputs(
    x_name: &str,
    x: &[f32],
    y_name: &str,
    y: &[f32],
    config: &CcmConfig,
) -> Result<usize> {
    if x_name.trim().is_empty() || y_name.trim().is_empty() || x_name == y_name {
        return Err(CalyxError::assay_insufficient_samples(
            "CCM series names must be non-empty and distinct",
        ));
    }
    if x.len() != y.len() {
        return Err(CalyxError::assay_insufficient_samples(format!(
            "CCM requires equal-length x/y series: x={} y={}",
            x.len(),
            y.len()
        )));
    }
    if config.embedding_dim < 2 {
        return Err(CalyxError::assay_insufficient_samples(format!(
            "CCM embedding_dim must be at least 2; got {}",
            config.embedding_dim
        )));
    }
    if config.tau == 0 {
        return Err(CalyxError::assay_insufficient_samples(
            "CCM tau must be at least 1",
        ));
    }
    if !config.min_convergence_delta.is_finite() || config.min_convergence_delta < 0.0 {
        return Err(CalyxError::assay_insufficient_samples(format!(
            "CCM min_convergence_delta must be finite and non-negative; got {}",
            config.min_convergence_delta
        )));
    }
    if !config.min_skill_gap.is_finite() || config.min_skill_gap < 0.0 {
        return Err(CalyxError::assay_insufficient_samples(format!(
            "CCM min_skill_gap must be finite and non-negative; got {}",
            config.min_skill_gap
        )));
    }
    for (name, values) in [(x_name, x), (y_name, y)] {
        for (idx, value) in values.iter().enumerate() {
            if !value.is_finite() {
                return Err(CalyxError::assay_insufficient_samples(format!(
                    "CCM {name}[{idx}] is not finite ({value})"
                )));
            }
        }
    }

    let start = (config.embedding_dim - 1)
        .checked_mul(config.tau)
        .ok_or_else(|| CalyxError::assay_insufficient_samples("CCM embedding lag overflow"))?;
    if start >= x.len() {
        return Err(CalyxError::assay_insufficient_samples(format!(
            "CCM embedding requires more samples than available: start={start} n={}",
            x.len()
        )));
    }
    let effective_points = x.len() - start;
    if config.library_sizes.len() < 2 {
        return Err(CalyxError::assay_insufficient_samples(
            "CCM requires at least two increasing library sizes",
        ));
    }
    let neighbor_count = config.embedding_dim + 1;
    let mut prev = 0usize;
    for &library_size in &config.library_sizes {
        if library_size <= neighbor_count {
            return Err(CalyxError::assay_insufficient_samples(format!(
                "CCM library size {library_size} must exceed neighbor_count {neighbor_count}"
            )));
        }
        if library_size > effective_points {
            return Err(CalyxError::assay_insufficient_samples(format!(
                "CCM library size {library_size} exceeds effective points {effective_points}"
            )));
        }
        if prev != 0 && library_size <= prev {
            return Err(CalyxError::assay_insufficient_samples(
                "CCM library sizes must be strictly increasing",
            ));
        }
        prev = library_size;
    }
    Ok(effective_points)
}

fn cross_map_direction(
    manifold_name: &str,
    manifold: &[f32],
    target_name: &str,
    target: &[f32],
    embedding_dim: usize,
    tau: usize,
    library_sizes: &[usize],
) -> Result<CcmDirectionReport> {
    let start = (embedding_dim - 1) * tau;
    let embedding = delay_embedding(manifold, embedding_dim, tau);
    let aligned_target: Vec<f32> = target[start..].to_vec();
    let mut library_skills = Vec::with_capacity(library_sizes.len());
    for &library_size in library_sizes {
        library_skills.push(CcmLibrarySkill {
            library_size,
            rho: simplex_skill(&embedding, &aligned_target, embedding_dim + 1, library_size)?,
        });
    }
    let first = library_skills[0].rho;
    let final_rho = library_skills[library_skills.len() - 1].rho;
    Ok(CcmDirectionReport {
        manifold: manifold_name.to_string(),
        target: target_name.to_string(),
        library_skills,
        final_rho,
        convergence_delta: final_rho - first,
    })
}

fn delay_embedding(values: &[f32], embedding_dim: usize, tau: usize) -> Vec<Vec<f32>> {
    let start = (embedding_dim - 1) * tau;
    let mut out = Vec::with_capacity(values.len() - start);
    for t in start..values.len() {
        let mut point = Vec::with_capacity(embedding_dim);
        for lag in 0..embedding_dim {
            point.push(values[t - lag * tau]);
        }
        out.push(point);
    }
    out
}

fn simplex_skill(
    embedding: &[Vec<f32>],
    target: &[f32],
    neighbor_count: usize,
    library_size: usize,
) -> Result<f32> {
    let mut predictions = Vec::with_capacity(library_size);
    let mut actual = Vec::with_capacity(library_size);
    for i in 0..library_size {
        let mut distances = Vec::with_capacity(library_size - 1);
        for j in 0..library_size {
            if i != j {
                distances.push((euclidean_distance(&embedding[i], &embedding[j]), j));
            }
        }
        distances.select_nth_unstable_by(neighbor_count - 1, |a, b| a.0.total_cmp(&b.0));
        distances[..neighbor_count].sort_by(|a, b| a.0.total_cmp(&b.0));
        let nearest = &distances[..neighbor_count];
        let prediction = simplex_prediction(nearest, target)?;
        predictions.push(prediction);
        actual.push(target[i]);
    }
    pearson_r(&predictions, &actual)
}

fn simplex_prediction(nearest: &[(f64, usize)], target: &[f32]) -> Result<f32> {
    const EPS: f64 = 1e-12;
    let d1 = nearest[0].0;
    if d1 <= EPS {
        let mut sum = 0.0f64;
        let mut count = 0usize;
        for &(distance, idx) in nearest {
            if distance <= EPS {
                sum += target[idx] as f64;
                count += 1;
            }
        }
        if count == 0 {
            return Err(CalyxError::assay_degenerate_input(
                "CCM nearest-neighbor zero-distance branch had no zero-distance neighbors",
            ));
        }
        return Ok((sum / count as f64) as f32);
    }

    let mut weighted_sum = 0.0f64;
    let mut weight_sum = 0.0f64;
    for &(distance, idx) in nearest {
        let weight = (-distance / d1).exp();
        weighted_sum += weight * target[idx] as f64;
        weight_sum += weight;
    }
    if weight_sum <= 0.0 || !weight_sum.is_finite() {
        return Err(CalyxError::assay_degenerate_input(
            "CCM simplex weights are degenerate",
        ));
    }
    Ok((weighted_sum / weight_sum) as f32)
}

fn euclidean_distance(a: &[f32], b: &[f32]) -> f64 {
    a.iter()
        .zip(b)
        .map(|(&x, &y)| {
            let d = x as f64 - y as f64;
            d * d
        })
        .sum::<f64>()
        .sqrt()
}

fn pearson_r(x: &[f32], y: &[f32]) -> Result<f32> {
    let n = x.len() as f64;
    let mean_x = x.iter().map(|&v| v as f64).sum::<f64>() / n;
    let mean_y = y.iter().map(|&v| v as f64).sum::<f64>() / n;
    let mut cov = 0.0f64;
    let mut var_x = 0.0f64;
    let mut var_y = 0.0f64;
    for (&xv, &yv) in x.iter().zip(y) {
        let dx = xv as f64 - mean_x;
        let dy = yv as f64 - mean_y;
        cov += dx * dy;
        var_x += dx * dx;
        var_y += dy * dy;
    }
    if var_x <= 0.0 || var_y <= 0.0 {
        return Err(CalyxError::assay_degenerate_input(
            "CCM cross-map skill undefined: predictions or targets are constant",
        ));
    }
    Ok((cov / (var_x.sqrt() * var_y.sqrt())).clamp(-1.0, 1.0) as f32)
}

fn ccm_verdict(
    x_to_y: &CcmDirectionReport,
    y_to_x: &CcmDirectionReport,
    min_convergence_delta: f32,
    min_skill_gap: f32,
) -> CcmVerdict {
    let x_to_y_converged = x_to_y.convergence_delta >= min_convergence_delta;
    let y_to_x_converged = y_to_x.convergence_delta >= min_convergence_delta;
    let y_to_x_gap = y_to_x.final_rho - x_to_y.final_rho;
    let x_to_y_gap = -y_to_x_gap;

    if y_to_x_converged && y_to_x_gap >= min_skill_gap {
        CcmVerdict::XCausesY
    } else if x_to_y_converged && x_to_y_gap >= min_skill_gap {
        CcmVerdict::YCausesX
    } else if x_to_y_converged && y_to_x_converged {
        CcmVerdict::Bidirectional
    } else {
        CcmVerdict::Inconclusive
    }
}
