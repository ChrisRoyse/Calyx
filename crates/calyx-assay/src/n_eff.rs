//! Effective-rank reporting for panel redundancy.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NeffReport {
    pub n_eff: f32,
    pub trace: f32,
    pub frobenius_sq: f32,
}

pub fn stable_rank(matrix: &[Vec<f32>]) -> NeffReport {
    let trace: f32 = matrix
        .iter()
        .enumerate()
        .map(|(index, row)| row.get(index).copied().unwrap_or(0.0))
        .sum();
    let frobenius_sq: f32 = matrix.iter().flatten().map(|value| value * value).sum();
    NeffReport {
        n_eff: if frobenius_sq > 0.0 {
            trace * trace / frobenius_sq
        } else {
            0.0
        },
        trace,
        frobenius_sq,
    }
}
