//! Batched exact cosine kNN over bounded corpus chunks.

#[cfg(not(feature = "cuda"))]
use std::cmp::Ordering;

#[cfg(feature = "cuda")]
use calyx_sextant::index::cuvs_chunked_bruteforce_topk;
use calyx_sextant::index::{
    CUVS_CHUNKED_EXACT_MAX_K, CuvsChunkedExactReport, CuvsChunkedExactRequest, CuvsDistanceMetric,
};
use serde::Serialize;

use crate::error::{PolyError, Result};

pub const ERR_EXACT_KNN_INVALID_REQUEST: &str = "CALYX_POLY_EXACT_KNN_INVALID_REQUEST";
pub const EXACT_KNN_MAX_DEVICE_K: usize = CUVS_CHUNKED_EXACT_MAX_K;
pub const EXACT_KNN_DEFAULT_QUERY_BATCH_ROWS: usize = 256;
pub const EXACT_KNN_STAGING_BUDGET_BYTES: usize = 256 * 1024 * 1024;
pub(crate) const EXACT_KNN_RERANK_GUARD_ROWS: usize = 32;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct ExactKnnConfig {
    pub k: usize,
    pub query_batch_rows: usize,
    pub corpus_chunk_rows: usize,
}

impl ExactKnnConfig {
    pub fn bounded(k: usize, corpus_rows: usize, dim: usize) -> Self {
        let query_batch_rows = EXACT_KNN_DEFAULT_QUERY_BATCH_ROWS;
        let boundary_rows = query_batch_rows.min(16);
        let bytes_per_row = dim
            .saturating_mul(size_of::<f32>())
            .saturating_add(boundary_rows.saturating_mul(size_of::<f32>()))
            .max(1);
        let corpus_chunk_rows = corpus_rows
            .min(EXACT_KNN_STAGING_BUDGET_BYTES / bytes_per_row)
            .max(1);
        Self {
            k,
            query_batch_rows,
            corpus_chunk_rows,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct ExactKnnExecution {
    pub backend: &'static str,
    pub cuda_compiled: bool,
    pub corpus_rows: usize,
    pub query_count: usize,
    pub output_k: usize,
    pub device_k: usize,
    pub query_batch_rows: usize,
    pub query_batches: usize,
    pub corpus_chunk_rows: usize,
    pub exhaustive_cpu_similarity_evaluations: u64,
    pub shortlist_cpu_similarity_evaluations: u64,
    pub cuda_reports: Vec<CuvsChunkedExactReport>,
}

#[derive(Clone, Debug)]
pub struct ExactKnnBatch {
    pub rankings: Vec<Vec<usize>>,
    pub execution: ExactKnnExecution,
}

pub fn exact_cosine_top_k(
    corpus: &[&[f32]],
    queries: &[&[f32]],
    k: usize,
    excluded_corpus_rows: Option<&[usize]>,
) -> Result<ExactKnnBatch> {
    let dim = corpus.first().map_or(0, |row| row.len());
    exact_cosine_top_k_with_config(
        corpus,
        queries,
        excluded_corpus_rows,
        ExactKnnConfig::bounded(k, corpus.len(), dim),
    )
}

pub fn exact_cosine_top_k_with_config(
    corpus: &[&[f32]],
    queries: &[&[f32]],
    excluded_corpus_rows: Option<&[usize]>,
    config: ExactKnnConfig,
) -> Result<ExactKnnBatch> {
    let dim = validate(corpus, queries, excluded_corpus_rows, config)?;
    let device_k = config.k + usize::from(excluded_corpus_rows.is_some());
    #[cfg(feature = "cuda")]
    let (rankings, reports, cpu_evaluations) =
        run_cuda(corpus, queries, excluded_corpus_rows, config, dim, device_k)?;
    #[cfg(not(feature = "cuda"))]
    let _ = dim;
    #[cfg(not(feature = "cuda"))]
    let (rankings, reports, cpu_evaluations) =
        run_cpu_reference(corpus, queries, excluded_corpus_rows, config.k);
    Ok(ExactKnnBatch {
        rankings,
        execution: ExactKnnExecution {
            backend: if cfg!(feature = "cuda") {
                "cuvs-bruteforce-chunked"
            } else {
                "cpu-reference-non-cuda-build"
            },
            cuda_compiled: calyx_sextant::CUVS_COMPILED,
            corpus_rows: corpus.len(),
            query_count: queries.len(),
            output_k: config.k,
            device_k,
            query_batch_rows: config.query_batch_rows,
            query_batches: queries.len().div_ceil(config.query_batch_rows),
            corpus_chunk_rows: config.corpus_chunk_rows.min(corpus.len()),
            exhaustive_cpu_similarity_evaluations: cpu_evaluations,
            shortlist_cpu_similarity_evaluations: 0,
            cuda_reports: reports,
        },
    })
}

pub fn exact_cosine_synthetic_probe(
    seed: u64,
    corpus_rows: u64,
    dim: usize,
    queries: &[f32],
    query_count: usize,
    k: usize,
    corpus_chunk_rows: usize,
) -> Result<CuvsChunkedExactReport> {
    Ok(
        calyx_sextant::index::cuvs_chunked_bruteforce_topk_synthetic(
            seed,
            CuvsChunkedExactRequest {
                corpus_rows,
                dim,
                queries,
                query_count,
                k,
                chunk_rows: corpus_chunk_rows,
                metric: CuvsDistanceMetric::Cosine,
            },
        )?
        .report,
    )
}

fn validate(
    corpus: &[&[f32]],
    queries: &[&[f32]],
    excluded: Option<&[usize]>,
    config: ExactKnnConfig,
) -> Result<usize> {
    let dim = corpus.first().map_or(0, |row| row.len());
    let device_k = config.k.saturating_add(usize::from(excluded.is_some()));
    let shape_invalid = corpus.is_empty()
        || queries.is_empty()
        || dim == 0
        || config.k == 0
        || config.query_batch_rows == 0
        || config.corpus_chunk_rows == 0
        || device_k > corpus.len()
        || (cfg!(feature = "cuda") && device_k > EXACT_KNN_MAX_DEVICE_K)
        || corpus
            .iter()
            .chain(queries.iter())
            .any(|row| row.len() != dim || row.iter().any(|value| !value.is_finite()));
    let exclusions_invalid = excluded.is_some_and(|rows| {
        rows.len() != queries.len() || rows.iter().any(|row| *row >= corpus.len())
    });
    if shape_invalid || exclusions_invalid {
        return Err(PolyError::diagnostics(
            ERR_EXACT_KNN_INVALID_REQUEST,
            format!(
                "exact kNN requires finite equal-dimension rows, non-empty queries, 0<k(+exclusion)<={EXACT_KNN_MAX_DEVICE_K}, and positive batch/chunk sizes"
            ),
        ));
    }
    Ok(dim)
}

#[cfg(feature = "cuda")]
fn run_cuda(
    corpus: &[&[f32]],
    queries: &[&[f32]],
    excluded: Option<&[usize]>,
    config: ExactKnnConfig,
    dim: usize,
    device_k: usize,
) -> Result<(Vec<Vec<usize>>, Vec<CuvsChunkedExactReport>, u64)> {
    let mut rankings = Vec::with_capacity(queries.len());
    let mut reports = Vec::new();
    for batch_start in (0..queries.len()).step_by(config.query_batch_rows) {
        let batch_end = (batch_start + config.query_batch_rows).min(queries.len());
        let mut query_values = Vec::with_capacity((batch_end - batch_start) * dim);
        for query in &queries[batch_start..batch_end] {
            query_values.extend_from_slice(query);
        }
        let topk = cuvs_chunked_bruteforce_topk(
            CuvsChunkedExactRequest {
                corpus_rows: corpus.len() as u64,
                dim,
                queries: &query_values,
                query_count: batch_end - batch_start,
                k: device_k,
                chunk_rows: config.corpus_chunk_rows,
                metric: CuvsDistanceMetric::Cosine,
            },
            |start, take, output| {
                let start = usize::try_from(start).expect("validated corpus row offset");
                for (target, source) in output
                    .chunks_exact_mut(dim)
                    .zip(&corpus[start..start + take])
                {
                    target.copy_from_slice(source);
                }
                Ok(())
            },
        )?;
        for local_query in 0..topk.query_count {
            let (ids, _) = topk.row(local_query);
            let excluded_row = excluded.map(|rows| rows[batch_start + local_query]);
            rankings.push(
                ids.iter()
                    .map(|id| *id as usize)
                    .filter(|id| Some(*id) != excluded_row)
                    .take(config.k)
                    .collect(),
            );
        }
        reports.push(topk.report);
    }
    Ok((rankings, reports, 0))
}

#[cfg(not(feature = "cuda"))]
fn run_cpu_reference(
    corpus: &[&[f32]],
    queries: &[&[f32]],
    excluded: Option<&[usize]>,
    k: usize,
) -> (Vec<Vec<usize>>, Vec<CuvsChunkedExactReport>, u64) {
    let rankings = queries
        .iter()
        .enumerate()
        .map(|(query_idx, query)| {
            let excluded_row = excluded.map(|rows| rows[query_idx]);
            let mut scored = corpus
                .iter()
                .enumerate()
                .filter(|(idx, _)| Some(*idx) != excluded_row)
                .map(|(idx, row)| (idx, cosine_distance(query, row)))
                .collect::<Vec<_>>();
            scored.sort_by(|left, right| {
                left.1
                    .partial_cmp(&right.1)
                    .unwrap_or(Ordering::Equal)
                    .then_with(|| left.0.cmp(&right.0))
            });
            scored.into_iter().take(k).map(|row| row.0).collect()
        })
        .collect();
    (
        rankings,
        Vec::new(),
        (corpus.len() as u64).saturating_mul(queries.len() as u64),
    )
}

#[cfg(not(feature = "cuda"))]
fn cosine_distance(a: &[f32], b: &[f32]) -> f32 {
    let mut dot = 0.0f32;
    let mut norm_a = 0.0f32;
    let mut norm_b = 0.0f32;
    for (left, right) in a.iter().zip(b) {
        dot += left * right;
        norm_a += left * left;
        norm_b += right * right;
    }
    if norm_a == 0.0 || norm_b == 0.0 {
        1.0
    } else {
        1.0 - dot / (norm_a.sqrt() * norm_b.sqrt())
    }
}
