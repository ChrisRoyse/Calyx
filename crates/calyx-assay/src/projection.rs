//! Deterministic random projection pre-step for high-dimensional Assay inputs.

use calyx_core::{CalyxError, Result};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProjectionReport {
    pub input_rows: usize,
    pub input_dim: usize,
    pub output_dim: usize,
    pub projected: Vec<Vec<f32>>,
    pub seed: u64,
}

pub fn target_projection_dim(rows: usize, input_dim: usize) -> usize {
    if rows <= 1 {
        return input_dim.min(1);
    }
    let log2 = (rows as f64).log2().ceil() as usize;
    input_dim.min((2 * log2).max(1))
}

pub fn project_cpu(matrix: &[Vec<f32>], seed: u64) -> ProjectionReport {
    let input_dim = matrix.first().map_or(0, Vec::len);
    let output_dim = target_projection_dim(matrix.len(), input_dim);
    let scale = if output_dim == 0 {
        1.0
    } else {
        (output_dim as f32).sqrt()
    };
    let projected = matrix
        .iter()
        .map(|row| {
            (0..output_dim)
                .map(|out_col| {
                    row.iter()
                        .enumerate()
                        .map(|(in_col, value)| value * sign(seed, in_col, out_col) / scale)
                        .sum()
                })
                .collect()
        })
        .collect();
    ProjectionReport {
        input_rows: matrix.len(),
        input_dim,
        output_dim,
        projected,
        seed,
    }
}

pub fn project_gpu(_matrix: &[Vec<f32>], _seed: u64) -> Result<ProjectionReport> {
    Err(CalyxError::forge_device_unavailable(
        "Assay random projection has no Forge-backed GPU implementation; use project_cpu until PH28 GPU projection is implemented",
    ))
}

fn sign(seed: u64, in_col: usize, out_col: usize) -> f32 {
    let mut hasher = blake3::Hasher::new();
    hasher.update(&seed.to_be_bytes());
    hasher.update(&(in_col as u64).to_be_bytes());
    hasher.update(&(out_col as u64).to_be_bytes());
    if hasher.finalize().as_bytes()[0] & 1 == 0 {
        1.0
    } else {
        -1.0
    }
}
