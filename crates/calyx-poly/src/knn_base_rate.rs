//! kNN-of-resolved base rate (issue #81).
//!
//! For a live market, find its nearest **resolved** lookalikes and read off their empirical
//! YES-rate. CUDA builds use the shared bounded cuVS exact-kNN path; non-CUDA builds retain the
//! deterministic reference implementation for parity tests.

use calyx_core::CxId;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::error::{PolyError, Result};
use crate::exact_knn::{
    EXACT_KNN_MAX_DEVICE_K, EXACT_KNN_RERANK_GUARD_ROWS, ExactKnnExecution, exact_cosine_top_k,
};

/// The corpus was empty.
pub const ERR_KNN_EMPTY_CORPUS: &str = "CALYX_POLY_KNN_EMPTY_CORPUS";
/// `k` exceeded the corpus size, or was zero.
pub const ERR_KNN_K: &str = "CALYX_POLY_KNN_INVALID_K";
/// A vector dimension did not match the corpus dimension.
pub const ERR_KNN_DIM: &str = "CALYX_POLY_KNN_DIM_MISMATCH";
/// A vector contained a non-finite value.
pub const ERR_KNN_NON_FINITE: &str = "CALYX_POLY_KNN_NON_FINITE";
/// A neighbor id was missing from the outcome map (corpus/index inconsistency).
pub const ERR_KNN_MISSING_OUTCOME: &str = "CALYX_POLY_KNN_MISSING_OUTCOME";

/// A resolved market exemplar: its feature vector and whether the bought outcome resolved YES.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ResolvedExemplar {
    /// Content id of the resolved market observation.
    pub cx_id: CxId,
    /// The market's feature vector (same encoding as the query).
    pub vector: Vec<f32>,
    /// Whether the outcome resolved YES.
    pub outcome_yes: bool,
}

/// The kNN empirical base rate and its provenance.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct KnnBaseRate {
    /// Empirical YES-rate among the `k` nearest resolved lookalikes.
    pub p_yes: f64,
    /// Neighbors used.
    pub k: usize,
    /// Corpus size searched.
    pub n_corpus: usize,
    /// The neighbor ids (hex) with their cosine similarity and outcome, most-similar first.
    pub neighbors: Vec<KnnNeighbor>,
    /// Mean cosine similarity of the neighbors (clamped to `[0,1]`).
    pub mean_similarity: f64,
    /// Reliability weight for the blend: mean similarity × neighbor-count saturation.
    pub reliability: f64,
}

/// One resolved neighbor.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct KnnNeighbor {
    /// Neighbor content id (hex).
    pub cx_id: String,
    /// Cosine similarity to the query.
    pub similarity: f32,
    /// Neighbor's resolved outcome.
    pub outcome_yes: bool,
}

/// Computes the kNN-of-resolved base rate for `query` over `corpus`.
pub fn knn_base_rate(corpus: &[ResolvedExemplar], query: &[f32], k: usize) -> Result<KnnBaseRate> {
    knn_base_rate_with_execution(corpus, query, k).map(|result| result.0)
}

/// Computes the base rate and returns exact-kNN execution telemetry for FSV/readback.
pub fn knn_base_rate_with_execution(
    corpus: &[ResolvedExemplar],
    query: &[f32],
    k: usize,
) -> Result<(KnnBaseRate, ExactKnnExecution)> {
    if corpus.is_empty() {
        return Err(PolyError::diagnostics(
            ERR_KNN_EMPTY_CORPUS,
            "kNN base rate requires a non-empty resolved corpus",
        ));
    }
    if k == 0 || k > corpus.len() {
        return Err(PolyError::diagnostics(
            ERR_KNN_K,
            format!("kNN k={k} must be in 1..={}", corpus.len()),
        ));
    }
    let dim = corpus[0].vector.len();
    if dim == 0 {
        return Err(PolyError::diagnostics(
            ERR_KNN_DIM,
            "corpus vectors are empty",
        ));
    }
    if query.len() != dim {
        return Err(PolyError::diagnostics(
            ERR_KNN_DIM,
            format!("query dim {} != corpus dim {dim}", query.len()),
        ));
    }
    if query.iter().any(|v| !v.is_finite()) {
        return Err(PolyError::diagnostics(
            ERR_KNN_NON_FINITE,
            "query vector contains a non-finite value",
        ));
    }

    let mut unique_by_id: BTreeMap<CxId, &ResolvedExemplar> = BTreeMap::new();
    for ex in corpus {
        if ex.vector.len() != dim {
            return Err(PolyError::diagnostics(
                ERR_KNN_DIM,
                format!("exemplar {} dim {} != {dim}", ex.cx_id, ex.vector.len()),
            ));
        }
        if ex.vector.iter().any(|v| !v.is_finite()) {
            return Err(PolyError::diagnostics(
                ERR_KNN_NON_FINITE,
                format!("exemplar {} vector contains a non-finite value", ex.cx_id),
            ));
        }
        unique_by_id.insert(ex.cx_id, ex);
    }

    // HNSW insert replaced duplicate CxIds. Preserve that contract while avoiding an index build.
    let ordered = unique_by_id.values().copied().collect::<Vec<_>>();
    let needed = k.min(ordered.len());
    let candidate_k = if needed > EXACT_KNN_MAX_DEVICE_K {
        needed
    } else {
        needed
            .saturating_add(EXACT_KNN_RERANK_GUARD_ROWS)
            .min(ordered.len())
            .min(EXACT_KNN_MAX_DEVICE_K)
    };
    let corpus_vectors = ordered
        .iter()
        .map(|row| row.vector.as_slice())
        .collect::<Vec<_>>();
    let query_vectors = [query];
    let mut exact = exact_cosine_top_k(&corpus_vectors, &query_vectors, candidate_k, None)?;
    exact.execution.shortlist_cpu_similarity_evaluations = exact.rankings[0].len() as u64;
    let mut hits = exact.rankings[0]
        .iter()
        .map(|idx| (ordered[*idx], cosine(query, &ordered[*idx].vector)))
        .collect::<Vec<_>>();
    hits.sort_by(|left, right| {
        right
            .1
            .total_cmp(&left.1)
            .then_with(|| left.0.cx_id.to_string().cmp(&right.0.cx_id.to_string()))
    });
    hits.truncate(needed);
    let mut neighbors = Vec::with_capacity(hits.len());
    let mut yes = 0usize;
    let mut sim_sum = 0.0f64;
    for (row, sim) in &hits {
        if row.outcome_yes {
            yes += 1;
        }
        sim_sum += (*sim as f64).clamp(0.0, 1.0);
        neighbors.push(KnnNeighbor {
            cx_id: row.cx_id.to_string(),
            similarity: *sim,
            outcome_yes: row.outcome_yes,
        });
    }
    let k_used = neighbors.len();
    let p_yes = yes as f64 / k_used as f64;
    let mean_similarity = sim_sum / k_used as f64;
    // Reliability: similar neighbors + enough of them. Saturates the count at 20.
    let count_sat = (k_used as f64 / 20.0).min(1.0);
    let reliability = (mean_similarity * count_sat).clamp(0.0, 1.0);

    Ok((
        KnnBaseRate {
            p_yes,
            k: k_used,
            n_corpus: corpus.len(),
            neighbors,
            mean_similarity,
            reliability,
        },
        exact.execution,
    ))
}

fn cosine(a: &[f32], b: &[f32]) -> f32 {
    let mut dot = 0.0;
    let mut norm_a = 0.0;
    let mut norm_b = 0.0;
    for (left, right) in a.iter().zip(b) {
        dot += left * right;
        norm_a += left * left;
        norm_b += right * right;
    }
    if norm_a == 0.0 || norm_b == 0.0 {
        0.0
    } else {
        dot / (norm_a.sqrt() * norm_b.sqrt())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cx(i: u8) -> CxId {
        let mut b = [0u8; 16];
        b[15] = i;
        CxId::from_bytes(b)
    }

    #[test]
    fn base_rate_reflects_nearest_cluster() {
        // Two clusters: "up" markets near +x resolve YES, "down" markets near -x resolve NO.
        let mut corpus = Vec::new();
        for i in 0..10 {
            corpus.push(ResolvedExemplar {
                cx_id: cx(i),
                vector: vec![1.0 + 0.01 * i as f32, 0.0],
                outcome_yes: true,
            });
        }
        for i in 10..20 {
            corpus.push(ResolvedExemplar {
                cx_id: cx(i),
                vector: vec![-1.0 - 0.01 * i as f32, 0.0],
                outcome_yes: false,
            });
        }
        // A query near the "up" cluster.
        let up = knn_base_rate(&corpus, &[1.0, 0.0], 5).unwrap();
        assert!(
            (up.p_yes - 1.0).abs() < 1e-9,
            "up query → YES base rate, got {}",
            up.p_yes
        );
        assert!(up.mean_similarity > 0.9);
        // A query near the "down" cluster.
        let down = knn_base_rate(&corpus, &[-1.0, 0.0], 5).unwrap();
        assert!(
            (down.p_yes - 0.0).abs() < 1e-9,
            "down query → NO base rate, got {}",
            down.p_yes
        );
    }

    #[test]
    fn fails_closed_on_bad_inputs() {
        let corpus = vec![ResolvedExemplar {
            cx_id: cx(1),
            vector: vec![1.0, 0.0],
            outcome_yes: true,
        }];
        assert_eq!(
            knn_base_rate(&[], &[1.0, 0.0], 1).unwrap_err().code(),
            ERR_KNN_EMPTY_CORPUS
        );
        assert_eq!(
            knn_base_rate(&corpus, &[1.0, 0.0], 5).unwrap_err().code(),
            ERR_KNN_K
        );
        assert_eq!(
            knn_base_rate(&corpus, &[1.0], 1).unwrap_err().code(),
            ERR_KNN_DIM
        );
        assert_eq!(
            knn_base_rate(&corpus, &[f32::NAN, 0.0], 1)
                .unwrap_err()
                .code(),
            ERR_KNN_NON_FINITE
        );
    }
}
