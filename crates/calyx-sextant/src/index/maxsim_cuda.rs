//! CUDA MaxSim serving for persisted multi-vector indexes.

use calyx_core::Result;
use serde::Serialize;

#[cfg(sextant_cuvs)]
use crate::error::CALYX_INDEX_IO;
use crate::error::{CALYX_INDEX_INVALID_PARAMS, sextant_error};

pub const MAXSIM_CUDA_MAX_K: usize = 1024;
pub const MAXSIM_CUDA_MAX_BATCH: usize = 4096;

mod imp;

#[derive(Clone, Debug)]
pub struct MaxSimCudaRequest<'a> {
    pub token_dim: usize,
    pub total_rows: usize,
    pub total_tokens: usize,
    pub query_tokens: &'a [f32],
    pub query_token_count: usize,
    pub k: usize,
    pub chunk_rows: usize,
    pub chunk_tokens: usize,
}

#[derive(Clone, Debug)]
pub struct MaxSimCudaBatchRequest<'a> {
    pub token_dim: usize,
    pub total_rows: usize,
    pub total_tokens: usize,
    pub query_tokens: &'a [f32],
    /// Token offsets for each logical query, with a terminal offset. Offsets
    /// count tokens (not f32 components) and must be strictly increasing.
    pub query_offsets: &'a [u32],
    pub k: usize,
    pub chunk_rows: usize,
    pub chunk_tokens: usize,
}

#[derive(Debug)]
pub struct MaxSimCudaChunk {
    pub row_count: usize,
    pub token_count: usize,
    pub row_offsets: Vec<u32>,
    pub tokens: Vec<f32>,
    pub token_norms: Vec<f32>,
    pub id_hi: Vec<u64>,
    pub id_lo: Vec<u64>,
    pub candidate_mask: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct MaxSimCudaReport {
    pub backend: &'static str,
    pub total_rows: usize,
    pub total_tokens: usize,
    pub token_dim: usize,
    pub query_rows: usize,
    pub query_tokens: usize,
    pub k: usize,
    pub chunk_rows: usize,
    pub chunk_tokens: usize,
    pub chunks: usize,
    pub score_kernel_launches: usize,
    pub topk_kernel_launches: usize,
    pub merge_kernel_launches: usize,
    pub h2d_bytes: u64,
    pub d2h_bytes: u64,
    pub final_readback_pairs: usize,
    pub candidate_mask_uploaded: bool,
    pub host_merge: bool,
    pub peak_device_bytes: usize,
    pub elapsed_us: u128,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct MaxSimCudaTopK {
    pub id_hi: Vec<u64>,
    pub id_lo: Vec<u64>,
    pub scores: Vec<f32>,
    pub report: MaxSimCudaReport,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MaxSimCudaBatchRow {
    pub id_hi: Vec<u64>,
    pub id_lo: Vec<u64>,
    pub scores: Vec<f32>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MaxSimCudaBatchTopK {
    pub rows: Vec<MaxSimCudaBatchRow>,
    pub report: MaxSimCudaReport,
}

pub fn maxsim_cuda_topk<F>(
    request: MaxSimCudaRequest<'_>,
    mut load_chunk: F,
) -> Result<MaxSimCudaTopK>
where
    F: FnMut(usize, usize, usize) -> Result<Option<MaxSimCudaChunk>>,
{
    validate_request(&request)?;
    if request.total_rows == 0 || request.k == 0 {
        return Ok(MaxSimCudaTopK {
            id_hi: Vec::new(),
            id_lo: Vec::new(),
            scores: Vec::new(),
            report: empty_report(&request, 0),
        });
    }
    imp::run(request, &mut load_chunk)
}

pub fn maxsim_cuda_topk_batch<F>(
    request: MaxSimCudaBatchRequest<'_>,
    mut load_chunk: F,
) -> Result<MaxSimCudaBatchTopK>
where
    F: FnMut(usize, usize, usize) -> Result<Option<MaxSimCudaChunk>>,
{
    validate_batch_request(&request)?;
    if request.total_rows == 0 || request.k == 0 {
        let query_rows = request.query_offsets.len() - 1;
        return Ok(MaxSimCudaBatchTopK {
            rows: vec![
                MaxSimCudaBatchRow {
                    id_hi: Vec::new(),
                    id_lo: Vec::new(),
                    scores: Vec::new(),
                };
                query_rows
            ],
            report: empty_batch_report(&request, 0),
        });
    }
    imp::run_batch(request, &mut load_chunk)
}

fn validate_request(request: &MaxSimCudaRequest<'_>) -> Result<()> {
    if request.token_dim == 0
        || request.k == 0
        || request.k > MAXSIM_CUDA_MAX_K
        || request.chunk_rows == 0
        || request.chunk_tokens == 0
        || request.query_tokens.len() != request.query_token_count * request.token_dim
    {
        return Err(invalid(format!(
            "invalid MaxSim CUDA shape; require dim>0, 0<k<={MAXSIM_CUDA_MAX_K}, nonzero chunks, and query len == query_count*dim"
        )));
    }
    if request.query_tokens.iter().any(|value| !value.is_finite()) {
        return Err(invalid(
            "MaxSim CUDA query tokens contain non-finite values",
        ));
    }
    Ok(())
}

fn validate_batch_request(request: &MaxSimCudaBatchRequest<'_>) -> Result<()> {
    let query_rows = request.query_offsets.len().saturating_sub(1);
    let query_tokens = request
        .query_tokens
        .len()
        .checked_div(request.token_dim.max(1));
    if request.token_dim == 0
        || query_rows == 0
        || query_rows > MAXSIM_CUDA_MAX_BATCH
        || request.k == 0
        || request.k > MAXSIM_CUDA_MAX_K
        || request.chunk_rows == 0
        || request.chunk_tokens == 0
        || !request.query_tokens.len().is_multiple_of(request.token_dim)
        || request.query_offsets.first().copied() != Some(0)
        || request
            .query_offsets
            .last()
            .copied()
            .map(|value| value as usize)
            != query_tokens
        || request
            .query_offsets
            .windows(2)
            .any(|pair| pair[0] >= pair[1])
    {
        return Err(invalid(format!(
            "invalid batched MaxSim CUDA shape; require dim>0, 0<query_rows<={MAXSIM_CUDA_MAX_BATCH}, 0<k<={MAXSIM_CUDA_MAX_K}, nonempty query token ranges, and terminal offset == flattened token count"
        )));
    }
    if request.query_tokens.iter().any(|value| !value.is_finite()) {
        return Err(invalid(
            "batched MaxSim CUDA query tokens contain non-finite values",
        ));
    }
    Ok(())
}

#[cfg_attr(not(sextant_cuvs), allow(dead_code))]
fn validate_chunk(request: &MaxSimCudaRequest<'_>, chunk: &MaxSimCudaChunk) -> Result<()> {
    if chunk.row_count == 0 || chunk.row_count > request.chunk_rows {
        return Err(invalid("MaxSim CUDA chunk row count is out of range"));
    }
    if chunk.token_count > request.chunk_tokens {
        return Err(invalid(
            "MaxSim CUDA chunk token count exceeds request budget",
        ));
    }
    if chunk.row_offsets.len() != chunk.row_count + 1
        || chunk.row_offsets.first().copied() != Some(0)
        || chunk.row_offsets.last().copied() != Some(chunk.token_count as u32)
    {
        return Err(invalid("MaxSim CUDA chunk row offsets are invalid"));
    }
    for window in chunk.row_offsets.windows(2) {
        if window[1] < window[0] {
            return Err(invalid("MaxSim CUDA chunk row offsets are not monotonic"));
        }
    }
    if chunk.tokens.len() != chunk.token_count * request.token_dim
        || chunk.token_norms.len() != chunk.token_count
        || chunk.id_hi.len() != chunk.row_count
        || chunk.id_lo.len() != chunk.row_count
        || chunk.candidate_mask.len() != chunk.row_count
    {
        return Err(invalid("MaxSim CUDA chunk buffers do not match shape"));
    }
    if chunk
        .tokens
        .iter()
        .chain(chunk.token_norms.iter())
        .any(|value| !value.is_finite())
    {
        return Err(invalid("MaxSim CUDA chunk contains non-finite values"));
    }
    Ok(())
}

fn empty_report(request: &MaxSimCudaRequest<'_>, elapsed_us: u128) -> MaxSimCudaReport {
    MaxSimCudaReport {
        backend: "cuda-maxsim-chunked-v1",
        total_rows: request.total_rows,
        total_tokens: request.total_tokens,
        token_dim: request.token_dim,
        query_rows: 1,
        query_tokens: request.query_token_count,
        k: request.k,
        chunk_rows: request.chunk_rows,
        chunk_tokens: request.chunk_tokens,
        chunks: 0,
        score_kernel_launches: 0,
        topk_kernel_launches: 0,
        merge_kernel_launches: 0,
        h2d_bytes: 0,
        d2h_bytes: 0,
        final_readback_pairs: 0,
        candidate_mask_uploaded: false,
        host_merge: false,
        peak_device_bytes: 0,
        elapsed_us,
    }
}

fn empty_batch_report(request: &MaxSimCudaBatchRequest<'_>, elapsed_us: u128) -> MaxSimCudaReport {
    MaxSimCudaReport {
        backend: "cuda-maxsim-batched-chunked-v2",
        total_rows: request.total_rows,
        total_tokens: request.total_tokens,
        token_dim: request.token_dim,
        query_rows: request.query_offsets.len() - 1,
        query_tokens: request.query_tokens.len() / request.token_dim,
        k: request.k,
        chunk_rows: request.chunk_rows,
        chunk_tokens: request.chunk_tokens,
        chunks: 0,
        score_kernel_launches: 0,
        topk_kernel_launches: 0,
        merge_kernel_launches: 0,
        h2d_bytes: 0,
        d2h_bytes: 0,
        final_readback_pairs: 0,
        candidate_mask_uploaded: false,
        host_merge: false,
        peak_device_bytes: 0,
        elapsed_us,
    }
}

fn invalid(detail: impl std::fmt::Display) -> calyx_core::CalyxError {
    sextant_error(CALYX_INDEX_INVALID_PARAMS, detail.to_string())
}

#[cfg(sextant_cuvs)]
fn cuda_error(
    stage: &'static str,
) -> impl FnOnce(cudarc::driver::DriverError) -> calyx_core::CalyxError {
    move |error| sextant_error(CALYX_INDEX_IO, format!("MaxSim CUDA {stage}: {error}"))
}
