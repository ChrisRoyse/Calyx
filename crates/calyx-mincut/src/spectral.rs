use std::collections::BTreeMap;

use calyx_core::CxId;
use calyx_paths::AssocGraph;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::spectral_linalg::{column, lanczos_eigen};

pub type NodeId = CxId;
pub type SparseGraph = AssocGraph;
pub type SpectralResult<T> = std::result::Result<T, SpectralError>;

const EIGEN_EPS: f32 = 1.0e-6;
const DEFAULT_EIGEN_MAX_ITER: usize = 256;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EigenPair {
    pub eigenvalue: f32,
    pub eigenvector: Vec<f32>,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct SpectralCacheKey {
    pub scope: String,
    pub panel_version: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SpectralCacheEntry {
    pub centrality: Vec<(NodeId, f32)>,
    pub eigenpairs: Vec<EigenPair>,
    pub refreshed_at_seq: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SpectralCache {
    entries: BTreeMap<SpectralCacheKey, SpectralCacheEntry>,
}

impl SpectralCache {
    pub fn insert(&mut self, key: SpectralCacheKey, entry: SpectralCacheEntry) {
        self.entries.insert(key, entry);
    }

    pub fn get(&self, key: &SpectralCacheKey) -> Option<&SpectralCacheEntry> {
        self.entries.get(key)
    }

    pub fn invalidate(&mut self, key: &SpectralCacheKey) -> Option<SpectralCacheEntry> {
        self.entries.remove(key)
    }

    pub fn invalidate_scope(&mut self, scope: &str) {
        self.entries.retain(|key, _| key.scope != scope);
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[derive(Clone, Debug, PartialEq, Error)]
pub enum SpectralError {
    #[error(
        "CALYX_SPECTRAL_NOT_CONVERGED: spectral iteration did not converge after {iterations} iterations"
    )]
    NotConverged { iterations: usize },
    #[error("CALYX_SPECTRAL_GRAPH_TOO_SMALL: graph has {n} nodes, requires at least {required}")]
    GraphTooSmall { n: usize, required: usize },
    #[error("CALYX_SPECTRAL_SINGULAR_MATRIX: graph has no positive spectral mass")]
    SingularMatrix,
}

impl SpectralError {
    pub const fn code(&self) -> &'static str {
        match self {
            Self::NotConverged { .. } => "CALYX_SPECTRAL_NOT_CONVERGED",
            Self::GraphTooSmall { .. } => "CALYX_SPECTRAL_GRAPH_TOO_SMALL",
            Self::SingularMatrix => "CALYX_SPECTRAL_SINGULAR_MATRIX",
        }
    }
}

pub fn eigenvector_centrality(
    graph: &SparseGraph,
    max_iter: usize,
    tol: f32,
) -> SpectralResult<Vec<(NodeId, f32)>> {
    ensure_min_nodes(graph, 2)?;
    if max_iter == 0 {
        return Err(SpectralError::NotConverged { iterations: 0 });
    }
    let adjacency = sym_adjacency(graph);
    let n = adjacency.len();
    let mut current = vec![1.0 / (n as f32).sqrt(); n];
    let mut iterations = 0;

    for step in 1..=max_iter {
        iterations = step;
        let mut next = shifted_mat_vec(&adjacency, &current);
        normalize(&mut next)?;
        if l2_distance(&next, &current) < tol {
            return Ok(ranked_scores(graph, &next));
        }
        current = next;
    }
    Err(SpectralError::NotConverged { iterations })
}

pub fn laplacian_eigenmaps(graph: &SparseGraph, k: usize) -> SpectralResult<Vec<EigenPair>> {
    laplacian_eigenmaps_with_max_iter(graph, k, DEFAULT_EIGEN_MAX_ITER)
}

pub fn laplacian_eigenmaps_with_max_iter(
    graph: &SparseGraph,
    k: usize,
    max_iter: usize,
) -> SpectralResult<Vec<EigenPair>> {
    ensure_min_nodes(graph, 2)?;
    if k == 0 {
        return Ok(Vec::new());
    }
    if max_iter == 0 {
        return Err(SpectralError::NotConverged { iterations: 0 });
    }
    let laplacian = laplacian_matrix(graph);
    let (values, vectors) = lanczos_eigen(laplacian, max_iter)?;
    let mut pairs: Vec<_> = values
        .into_iter()
        .enumerate()
        .map(|(index, eigenvalue)| EigenPair {
            eigenvalue: clean_zero(eigenvalue),
            eigenvector: orient_vector(column(&vectors, index)),
        })
        .collect();
    pairs.sort_by(|left, right| left.eigenvalue.total_cmp(&right.eigenvalue));
    pairs.truncate(k.min(pairs.len()));
    Ok(pairs)
}

pub fn gft_project(signal: &[f32], eigenvectors: &[EigenPair]) -> Vec<f32> {
    eigenvectors
        .iter()
        .map(|pair| {
            assert_eq!(
                signal.len(),
                pair.eigenvector.len(),
                "GFT signal/eigenvector dimension mismatch"
            );
            dot(signal, &pair.eigenvector)
        })
        .collect()
}

pub fn gft_reconstruct(coefficients: &[f32], eigenvectors: &[EigenPair]) -> Vec<f32> {
    assert_eq!(
        coefficients.len(),
        eigenvectors.len(),
        "GFT coefficient/eigenvector count mismatch"
    );
    let Some(first) = eigenvectors.first() else {
        return Vec::new();
    };
    let mut signal = vec![0.0; first.eigenvector.len()];
    for (coefficient, pair) in coefficients.iter().zip(eigenvectors) {
        assert_eq!(
            signal.len(),
            pair.eigenvector.len(),
            "GFT eigenvector basis dimension mismatch"
        );
        for (dst, value) in signal.iter_mut().zip(&pair.eigenvector) {
            *dst += coefficient * value;
        }
    }
    signal
}

pub fn spectral_gap(eigenmaps: &[EigenPair]) -> f32 {
    if eigenmaps.len() < 2 {
        return 0.0;
    }
    (eigenmaps[1].eigenvalue - eigenmaps[0].eigenvalue).max(0.0)
}

fn ensure_min_nodes(graph: &SparseGraph, required: usize) -> SpectralResult<()> {
    let n = graph.node_count();
    if n < required {
        Err(SpectralError::GraphTooSmall { n, required })
    } else {
        Ok(())
    }
}

fn sym_adjacency(graph: &SparseGraph) -> Vec<Vec<f32>> {
    let n = graph.node_count();
    let mut matrix = vec![vec![0.0_f32; n]; n];
    for edge in graph.edges() {
        matrix[edge.src][edge.dst] = matrix[edge.src][edge.dst].max(edge.weight);
        matrix[edge.dst][edge.src] = matrix[edge.dst][edge.src].max(edge.weight);
    }
    matrix
}

fn laplacian_matrix(graph: &SparseGraph) -> Vec<Vec<f32>> {
    let adjacency = sym_adjacency(graph);
    let mut laplacian = vec![vec![0.0; adjacency.len()]; adjacency.len()];
    for (row, weights) in adjacency.iter().enumerate() {
        let degree = weights.iter().sum::<f32>();
        laplacian[row][row] = degree;
        for (col, weight) in weights.iter().enumerate() {
            if row != col {
                laplacian[row][col] = -*weight;
            }
        }
    }
    laplacian
}

fn ranked_scores(graph: &SparseGraph, vector: &[f32]) -> Vec<(NodeId, f32)> {
    let max = vector
        .iter()
        .map(|value| value.abs())
        .fold(0.0_f32, f32::max);
    let mut ranked: Vec<_> = vector
        .iter()
        .enumerate()
        .map(|(index, value)| {
            (
                graph.node_id(index).expect("spectral node id"),
                if max <= EIGEN_EPS {
                    0.0
                } else {
                    value.abs() / max
                },
            )
        })
        .collect();
    ranked.sort_by(|left, right| {
        right
            .1
            .total_cmp(&left.1)
            .then_with(|| left.0.as_bytes().cmp(right.0.as_bytes()))
    });
    ranked
}

fn orient_vector(mut vector: Vec<f32>) -> Vec<f32> {
    if let Some(first) = vector.iter().find(|value| value.abs() > EIGEN_EPS)
        && *first < 0.0
    {
        for value in &mut vector {
            *value = -*value;
        }
    }
    vector
}

fn shifted_mat_vec(matrix: &[Vec<f32>], vector: &[f32]) -> Vec<f32> {
    matrix
        .iter()
        .zip(vector)
        .map(|(row, identity)| dot(row, vector) + identity)
        .collect()
}

fn dot(left: &[f32], right: &[f32]) -> f32 {
    left.iter().zip(right).map(|(a, b)| a * b).sum()
}

fn l2_distance(left: &[f32], right: &[f32]) -> f32 {
    left.iter()
        .zip(right)
        .map(|(a, b)| (a - b).powi(2))
        .sum::<f32>()
        .sqrt()
}

fn normalize(vector: &mut [f32]) -> SpectralResult<()> {
    let norm = vector.iter().map(|value| value * value).sum::<f32>().sqrt();
    if !norm.is_finite() || norm <= EIGEN_EPS {
        return Err(SpectralError::SingularMatrix);
    }
    for value in vector {
        *value /= norm;
    }
    Ok(())
}

fn clean_zero(value: f32) -> f32 {
    if value.abs() < EIGEN_EPS { 0.0 } else { value }
}

// IMPORTANT: spectral centrality is structure-only; the MFVS kernel is outcome-anchored (A2).
// Centrality proposes candidates; grounding through oracle anchors confirms them.
