#[cfg(sextant_cuvs)]
use std::time::Instant;

use calyx_core::Result;

#[cfg(sextant_cuvs)]
use super::MaxSimCudaBatchRow;
use super::{
    MaxSimCudaBatchRequest, MaxSimCudaBatchTopK, MaxSimCudaChunk, MaxSimCudaRequest, MaxSimCudaTopK,
};

#[cfg(sextant_cuvs)]
use super::{MAXSIM_CUDA_MAX_K, MaxSimCudaReport, cuda_error, invalid, validate_chunk};

#[cfg(sextant_cuvs)]
use cudarc::driver::{
    CudaFunction, CudaSlice, CudaStream, LaunchConfig, PushKernelArg, ValidAsZeroBits,
};

#[cfg(sextant_cuvs)]
const CUBIN: &[u8] = include_bytes!(env!("SEXTANT_MAXSIM_CUBIN_PATH"));
#[cfg(sextant_cuvs)]
const THREADS: usize = 256;

#[path = "imp/runtime.rs"]
mod runtime;

#[cfg(sextant_cuvs)]
pub(super) fn run<F>(request: MaxSimCudaRequest<'_>, load_chunk: &mut F) -> Result<MaxSimCudaTopK>
where
    F: FnMut(usize, usize, usize) -> Result<Option<MaxSimCudaChunk>>,
{
    let started = Instant::now();
    let runtime = runtime::get()?;
    let stream = &runtime.stream;

    let query_norms = query_norms(&request);
    let query_tokens = stream
        .clone_htod(request.query_tokens)
        .map_err(cuda_error("query upload"))?;
    let query_norms_dev = stream
        .clone_htod(&query_norms)
        .map_err(cuda_error("query norm upload"))?;
    let mut global_hi = alloc_device::<u64>(stream, request.k, "global hi")?;
    let mut global_lo = alloc_device::<u64>(stream, request.k, "global lo")?;
    let mut global_scores = alloc_device::<f32>(stream, request.k, "global scores")?;
    let mut global_count = stream
        .clone_htod(&[0_u32])
        .map_err(cuda_error("global count init"))?;

    let mut state = RunState::new(&request, query_norms.len());
    while state.rows_seen < request.total_rows {
        let Some(chunk) = load_chunk(state.rows_seen, request.chunk_rows, request.chunk_tokens)?
        else {
            break;
        };
        validate_chunk(&request, &chunk)?;
        run_chunk(
            stream,
            &runtime.score_fn,
            &runtime.topk_fn,
            &runtime.merge_fn,
            &request,
            &query_tokens,
            &query_norms_dev,
            chunk,
            &mut global_hi,
            &mut global_lo,
            &mut global_scores,
            &mut global_count,
            &mut state,
        )?;
    }
    if state.rows_seen != request.total_rows || state.tokens_seen != request.total_tokens {
        return Err(invalid(format!(
            "MaxSim CUDA loader yielded rows/tokens {}/{}, expected {}/{}",
            state.rows_seen, state.tokens_seen, request.total_rows, request.total_tokens
        )));
    }

    let count = stream
        .clone_dtoh(&global_count)
        .map_err(cuda_error("global count readback"))?
        .into_iter()
        .next()
        .unwrap_or(0) as usize;
    let mut id_hi = stream
        .clone_dtoh(&global_hi)
        .map_err(cuda_error("final hi readback"))?;
    let mut id_lo = stream
        .clone_dtoh(&global_lo)
        .map_err(cuda_error("final lo readback"))?;
    let mut scores = stream
        .clone_dtoh(&global_scores)
        .map_err(cuda_error("final score readback"))?;
    id_hi.truncate(count);
    id_lo.truncate(count);
    scores.truncate(count);
    let d2h_bytes =
        (size_of::<u32>() + request.k * (size_of::<u64>() * 2 + size_of::<f32>())) as u64;
    Ok(MaxSimCudaTopK {
        id_hi,
        id_lo,
        scores,
        report: MaxSimCudaReport {
            backend: "cuda-maxsim-chunked-v1",
            total_rows: request.total_rows,
            total_tokens: request.total_tokens,
            token_dim: request.token_dim,
            query_rows: 1,
            query_tokens: request.query_token_count,
            k: request.k,
            chunk_rows: request.chunk_rows,
            chunk_tokens: request.chunk_tokens,
            chunks: state.chunks,
            score_kernel_launches: state.chunks,
            topk_kernel_launches: state.chunks,
            merge_kernel_launches: state.chunks,
            h2d_bytes: state.h2d_bytes,
            d2h_bytes,
            final_readback_pairs: count,
            candidate_mask_uploaded: true,
            host_merge: false,
            peak_device_bytes: state.peak_device_bytes,
            elapsed_us: started.elapsed().as_micros(),
        },
    })
}

#[cfg(sextant_cuvs)]
pub(super) fn run_batch<F>(
    request: MaxSimCudaBatchRequest<'_>,
    load_chunk: &mut F,
) -> Result<MaxSimCudaBatchTopK>
where
    F: FnMut(usize, usize, usize) -> Result<Option<MaxSimCudaChunk>>,
{
    let started = Instant::now();
    let runtime = runtime::get()?;
    let stream = &runtime.stream;
    let query_rows = request.query_offsets.len() - 1;
    let query_token_count = request.query_tokens.len() / request.token_dim;
    let topk_cells = query_rows
        .checked_mul(request.k)
        .ok_or_else(|| invalid("batched MaxSim CUDA top-k shape overflow"))?;

    let query_norms = flat_query_norms(request.query_tokens, request.token_dim);
    let query_tokens = stream
        .clone_htod(request.query_tokens)
        .map_err(cuda_error("batch query upload"))?;
    let query_norms_dev = stream
        .clone_htod(&query_norms)
        .map_err(cuda_error("batch query norm upload"))?;
    let query_offsets = stream
        .clone_htod(request.query_offsets)
        .map_err(cuda_error("batch query offset upload"))?;
    let mut global_hi = alloc_device::<u64>(stream, topk_cells, "batch global hi")?;
    let mut global_lo = alloc_device::<u64>(stream, topk_cells, "batch global lo")?;
    let mut global_scores = alloc_device::<f32>(stream, topk_cells, "batch global scores")?;
    let mut global_count = alloc_device::<u32>(stream, query_rows, "batch global count")?;

    let mut state = BatchRunState::new(&request, query_norms.len(), topk_cells);
    while state.rows_seen < request.total_rows {
        let Some(chunk) = load_chunk(state.rows_seen, request.chunk_rows, request.chunk_tokens)?
        else {
            break;
        };
        validate_batch_chunk(&request, &chunk)?;
        run_chunk_batch(
            stream,
            &runtime.batch_score_fn,
            &runtime.batch_topk_fn,
            &runtime.batch_merge_fn,
            &request,
            &query_tokens,
            &query_norms_dev,
            &query_offsets,
            chunk,
            &mut global_hi,
            &mut global_lo,
            &mut global_scores,
            &mut global_count,
            &mut state,
        )?;
    }
    if state.rows_seen != request.total_rows || state.tokens_seen != request.total_tokens {
        return Err(invalid(format!(
            "batched MaxSim CUDA loader yielded rows/tokens {}/{}, expected {}/{}",
            state.rows_seen, state.tokens_seen, request.total_rows, request.total_tokens
        )));
    }

    let counts = stream
        .clone_dtoh(&global_count)
        .map_err(cuda_error("batch global count readback"))?;
    let id_hi = stream
        .clone_dtoh(&global_hi)
        .map_err(cuda_error("batch final hi readback"))?;
    let id_lo = stream
        .clone_dtoh(&global_lo)
        .map_err(cuda_error("batch final lo readback"))?;
    let scores = stream
        .clone_dtoh(&global_scores)
        .map_err(cuda_error("batch final score readback"))?;
    let mut rows = Vec::with_capacity(query_rows);
    let mut final_pairs = 0usize;
    for (query_row, &count) in counts.iter().enumerate() {
        let count = count as usize;
        if count > request.k {
            return Err(invalid(format!(
                "batched MaxSim CUDA query {query_row} returned count {count} above k {}",
                request.k
            )));
        }
        let start = query_row * request.k;
        let end = start + count;
        final_pairs += count;
        rows.push(MaxSimCudaBatchRow {
            id_hi: id_hi[start..end].to_vec(),
            id_lo: id_lo[start..end].to_vec(),
            scores: scores[start..end].to_vec(),
        });
    }
    let d2h_bytes = (query_rows * size_of::<u32>()
        + topk_cells * (size_of::<u64>() * 2 + size_of::<f32>())) as u64;
    Ok(MaxSimCudaBatchTopK {
        rows,
        report: MaxSimCudaReport {
            backend: "cuda-maxsim-batched-chunked-v2",
            total_rows: request.total_rows,
            total_tokens: request.total_tokens,
            token_dim: request.token_dim,
            query_rows,
            query_tokens: query_token_count,
            k: request.k,
            chunk_rows: request.chunk_rows,
            chunk_tokens: request.chunk_tokens,
            chunks: state.chunks,
            score_kernel_launches: state.chunks,
            topk_kernel_launches: state.chunks,
            merge_kernel_launches: state.chunks,
            h2d_bytes: state.h2d_bytes,
            d2h_bytes,
            final_readback_pairs: final_pairs,
            candidate_mask_uploaded: true,
            host_merge: false,
            peak_device_bytes: state.peak_device_bytes,
            elapsed_us: started.elapsed().as_micros(),
        },
    })
}

#[cfg(sextant_cuvs)]
fn validate_batch_chunk(
    request: &MaxSimCudaBatchRequest<'_>,
    chunk: &MaxSimCudaChunk,
) -> Result<()> {
    let single = MaxSimCudaRequest {
        token_dim: request.token_dim,
        total_rows: request.total_rows,
        total_tokens: request.total_tokens,
        query_tokens: request.query_tokens,
        query_token_count: request.query_tokens.len() / request.token_dim,
        k: request.k,
        chunk_rows: request.chunk_rows,
        chunk_tokens: request.chunk_tokens,
    };
    validate_chunk(&single, chunk)
}

#[cfg(sextant_cuvs)]
#[allow(clippy::too_many_arguments)]
fn run_chunk(
    stream: &std::sync::Arc<CudaStream>,
    score_fn: &CudaFunction,
    topk_fn: &CudaFunction,
    merge_fn: &CudaFunction,
    request: &MaxSimCudaRequest<'_>,
    query_tokens: &CudaSlice<f32>,
    query_norms_dev: &CudaSlice<f32>,
    chunk: MaxSimCudaChunk,
    global_hi: &mut CudaSlice<u64>,
    global_lo: &mut CudaSlice<u64>,
    global_scores: &mut CudaSlice<f32>,
    global_count: &mut CudaSlice<u32>,
    state: &mut RunState,
) -> Result<()> {
    state.rows_seen += chunk.row_count;
    state.tokens_seen += chunk.token_count;
    let row_offsets = stream
        .clone_htod(&chunk.row_offsets)
        .map_err(cuda_error("row offset upload"))?;
    let tokens = stream
        .clone_htod(&chunk.tokens)
        .map_err(cuda_error("token upload"))?;
    let token_norms = stream
        .clone_htod(&chunk.token_norms)
        .map_err(cuda_error("token norm upload"))?;
    let id_hi = stream
        .clone_htod(&chunk.id_hi)
        .map_err(cuda_error("id hi upload"))?;
    let id_lo = stream
        .clone_htod(&chunk.id_lo)
        .map_err(cuda_error("id lo upload"))?;
    let candidate_mask = stream
        .clone_htod(&chunk.candidate_mask)
        .map_err(cuda_error("candidate upload"))?;
    let mut row_scores = alloc_device::<f32>(stream, chunk.row_count, "row scores")?;
    let mut chunk_hi = alloc_device::<u64>(stream, request.k, "chunk hi")?;
    let mut chunk_lo = alloc_device::<u64>(stream, request.k, "chunk lo")?;
    let mut chunk_scores = alloc_device::<f32>(stream, request.k, "chunk scores")?;
    let mut chunk_count = stream
        .clone_htod(&[0_u32])
        .map_err(cuda_error("chunk count init"))?;

    launch_score(
        stream,
        score_fn,
        query_tokens,
        query_norms_dev,
        &tokens,
        &token_norms,
        &row_offsets,
        &candidate_mask,
        chunk.row_count,
        request.token_dim,
        request.query_token_count,
        &mut row_scores,
    )?;
    launch_topk(
        stream,
        topk_fn,
        &row_scores,
        &id_hi,
        &id_lo,
        &candidate_mask,
        chunk.row_count,
        request.k,
        &mut chunk_hi,
        &mut chunk_lo,
        &mut chunk_scores,
        &mut chunk_count,
    )?;
    launch_merge(
        stream,
        merge_fn,
        &chunk_hi,
        &chunk_lo,
        &chunk_scores,
        &chunk_count,
        global_hi,
        global_lo,
        global_scores,
        global_count,
        request.k,
    )?;
    state.record_chunk(&chunk, request);
    Ok(())
}

#[cfg(sextant_cuvs)]
#[allow(clippy::too_many_arguments)]
fn run_chunk_batch(
    stream: &std::sync::Arc<CudaStream>,
    score_fn: &CudaFunction,
    topk_fn: &CudaFunction,
    merge_fn: &CudaFunction,
    request: &MaxSimCudaBatchRequest<'_>,
    query_tokens: &CudaSlice<f32>,
    query_norms_dev: &CudaSlice<f32>,
    query_offsets: &CudaSlice<u32>,
    chunk: MaxSimCudaChunk,
    global_hi: &mut CudaSlice<u64>,
    global_lo: &mut CudaSlice<u64>,
    global_scores: &mut CudaSlice<f32>,
    global_count: &mut CudaSlice<u32>,
    state: &mut BatchRunState,
) -> Result<()> {
    state.rows_seen += chunk.row_count;
    state.tokens_seen += chunk.token_count;
    let query_rows = request.query_offsets.len() - 1;
    let score_cells = query_rows
        .checked_mul(chunk.row_count)
        .ok_or_else(|| invalid("batched MaxSim CUDA score shape overflow"))?;
    let topk_cells = query_rows
        .checked_mul(request.k)
        .ok_or_else(|| invalid("batched MaxSim CUDA chunk top-k shape overflow"))?;
    let row_offsets = stream
        .clone_htod(&chunk.row_offsets)
        .map_err(cuda_error("batch row offset upload"))?;
    let tokens = stream
        .clone_htod(&chunk.tokens)
        .map_err(cuda_error("batch token upload"))?;
    let token_norms = stream
        .clone_htod(&chunk.token_norms)
        .map_err(cuda_error("batch token norm upload"))?;
    let id_hi = stream
        .clone_htod(&chunk.id_hi)
        .map_err(cuda_error("batch id hi upload"))?;
    let id_lo = stream
        .clone_htod(&chunk.id_lo)
        .map_err(cuda_error("batch id lo upload"))?;
    let candidate_mask = stream
        .clone_htod(&chunk.candidate_mask)
        .map_err(cuda_error("batch candidate upload"))?;
    let mut row_scores = alloc_device::<f32>(stream, score_cells, "batch row scores")?;
    let mut chunk_hi = alloc_device::<u64>(stream, topk_cells, "batch chunk hi")?;
    let mut chunk_lo = alloc_device::<u64>(stream, topk_cells, "batch chunk lo")?;
    let mut chunk_scores = alloc_device::<f32>(stream, topk_cells, "batch chunk scores")?;
    let mut chunk_count = alloc_device::<u32>(stream, query_rows, "batch chunk count")?;

    launch_score_batch(
        stream,
        score_fn,
        query_tokens,
        query_norms_dev,
        query_offsets,
        &tokens,
        &token_norms,
        &row_offsets,
        &candidate_mask,
        chunk.row_count,
        request.token_dim,
        query_rows,
        &mut row_scores,
    )?;
    launch_topk_batch(
        stream,
        topk_fn,
        &row_scores,
        &id_hi,
        &id_lo,
        &candidate_mask,
        chunk.row_count,
        query_rows,
        request.k,
        &mut chunk_hi,
        &mut chunk_lo,
        &mut chunk_scores,
        &mut chunk_count,
    )?;
    launch_merge_batch(
        stream,
        merge_fn,
        &chunk_hi,
        &chunk_lo,
        &chunk_scores,
        &chunk_count,
        global_hi,
        global_lo,
        global_scores,
        global_count,
        query_rows,
        request.k,
    )?;
    state.record_chunk(&chunk, request, score_cells, topk_cells);
    Ok(())
}

#[cfg(sextant_cuvs)]
struct RunState {
    chunks: usize,
    rows_seen: usize,
    tokens_seen: usize,
    h2d_bytes: u64,
    peak_device_bytes: usize,
    resident_query_bytes: usize,
}

#[cfg(sextant_cuvs)]
struct BatchRunState {
    chunks: usize,
    rows_seen: usize,
    tokens_seen: usize,
    h2d_bytes: u64,
    peak_device_bytes: usize,
    resident_query_bytes: usize,
}

#[cfg(sextant_cuvs)]
impl BatchRunState {
    fn new(
        request: &MaxSimCudaBatchRequest<'_>,
        query_norm_count: usize,
        topk_cells: usize,
    ) -> Self {
        let query_rows = request.query_offsets.len() - 1;
        let resident_query_bytes = size_of_val(request.query_tokens)
            + query_norm_count * size_of::<f32>()
            + size_of_val(request.query_offsets)
            + topk_cells * (size_of::<u64>() * 2 + size_of::<f32>())
            + query_rows * size_of::<u32>();
        Self {
            chunks: 0,
            rows_seen: 0,
            tokens_seen: 0,
            h2d_bytes: resident_query_bytes as u64,
            peak_device_bytes: resident_query_bytes,
            resident_query_bytes,
        }
    }

    fn record_chunk(
        &mut self,
        chunk: &MaxSimCudaChunk,
        request: &MaxSimCudaBatchRequest<'_>,
        score_cells: usize,
        topk_cells: usize,
    ) {
        self.h2d_bytes += (chunk.row_offsets.len() * size_of::<u32>()
            + chunk.tokens.len() * size_of::<f32>()
            + chunk.token_norms.len() * size_of::<f32>()
            + chunk.id_hi.len() * size_of::<u64>()
            + chunk.id_lo.len() * size_of::<u64>()
            + chunk.candidate_mask.len()) as u64;
        let chunk_device_bytes = chunk.row_offsets.len() * size_of::<u32>()
            + chunk.tokens.len() * size_of::<f32>()
            + chunk.token_norms.len() * size_of::<f32>()
            + chunk.id_hi.len() * size_of::<u64>()
            + chunk.id_lo.len() * size_of::<u64>()
            + chunk.candidate_mask.len()
            + score_cells * size_of::<f32>()
            + topk_cells * (size_of::<u64>() * 2 + size_of::<f32>())
            + (topk_cells / request.k) * size_of::<u32>();
        self.peak_device_bytes = self
            .peak_device_bytes
            .max(self.resident_query_bytes + chunk_device_bytes);
        self.chunks += 1;
    }
}

#[cfg(sextant_cuvs)]
impl RunState {
    fn new(request: &MaxSimCudaRequest<'_>, query_norm_count: usize) -> Self {
        let resident_query_bytes = size_of_val(request.query_tokens)
            + query_norm_count * size_of::<f32>()
            + request.k * (size_of::<u64>() * 2 + size_of::<f32>())
            + size_of::<u32>();
        Self {
            chunks: 0,
            rows_seen: 0,
            tokens_seen: 0,
            h2d_bytes: (request.query_tokens.len() + query_norm_count) as u64 * 4 + 4,
            peak_device_bytes: resident_query_bytes,
            resident_query_bytes,
        }
    }

    fn record_chunk(&mut self, chunk: &MaxSimCudaChunk, request: &MaxSimCudaRequest<'_>) {
        self.h2d_bytes += (chunk.row_offsets.len() * size_of::<u32>()
            + chunk.tokens.len() * size_of::<f32>()
            + chunk.token_norms.len() * size_of::<f32>()
            + chunk.id_hi.len() * size_of::<u64>()
            + chunk.id_lo.len() * size_of::<u64>()
            + chunk.candidate_mask.len()
            + size_of::<u32>()) as u64;
        let chunk_device_bytes = chunk.row_offsets.len() * size_of::<u32>()
            + chunk.tokens.len() * size_of::<f32>()
            + chunk.token_norms.len() * size_of::<f32>()
            + chunk.id_hi.len() * size_of::<u64>()
            + chunk.id_lo.len() * size_of::<u64>()
            + chunk.candidate_mask.len()
            + chunk.row_count * size_of::<f32>()
            + request.k * (size_of::<u64>() * 2 + size_of::<f32>())
            + size_of::<u32>();
        self.peak_device_bytes = self
            .peak_device_bytes
            .max(self.resident_query_bytes + chunk_device_bytes);
        self.chunks += 1;
    }
}

#[cfg(sextant_cuvs)]
fn query_norms(request: &MaxSimCudaRequest<'_>) -> Vec<f32> {
    flat_query_norms(request.query_tokens, request.token_dim)
}

#[cfg(sextant_cuvs)]
fn flat_query_norms(query_tokens: &[f32], token_dim: usize) -> Vec<f32> {
    query_tokens
        .chunks_exact(token_dim)
        .map(|token| {
            let mut squared = 0.0_f32;
            for value in token {
                squared += value * value;
            }
            squared.sqrt()
        })
        .collect()
}

#[cfg(sextant_cuvs)]
fn load(
    module: &std::sync::Arc<cudarc::driver::CudaModule>,
    name: &'static str,
    stage: &'static str,
) -> Result<CudaFunction> {
    module.load_function(name).map_err(cuda_error(stage))
}

#[cfg(sextant_cuvs)]
#[allow(clippy::too_many_arguments)]
fn launch_score(
    stream: &std::sync::Arc<CudaStream>,
    function: &CudaFunction,
    query_tokens: &CudaSlice<f32>,
    query_norms: &CudaSlice<f32>,
    doc_tokens: &CudaSlice<f32>,
    doc_norms: &CudaSlice<f32>,
    row_offsets: &CudaSlice<u32>,
    candidate_mask: &CudaSlice<u8>,
    row_count: usize,
    token_dim: usize,
    query_count: usize,
    row_scores: &mut CudaSlice<f32>,
) -> Result<()> {
    let row_count_i32 = to_i32(row_count, "row count")?;
    let token_dim_i32 = to_i32(token_dim, "token dim")?;
    let query_count_i32 = to_i32(query_count, "query count")?;
    let mut launch = stream.launch_builder(function);
    unsafe {
        launch
            .arg(query_tokens)
            .arg(query_norms)
            .arg(&query_count_i32)
            .arg(doc_tokens)
            .arg(doc_norms)
            .arg(row_offsets)
            .arg(candidate_mask)
            .arg(&row_count_i32)
            .arg(&token_dim_i32)
            .arg(row_scores)
            .launch(row_block_config(row_count)?)
    }
    .map(|_| ())
    .map_err(cuda_error("score launch"))
}

#[cfg(sextant_cuvs)]
#[allow(clippy::too_many_arguments)]
fn launch_score_batch(
    stream: &std::sync::Arc<CudaStream>,
    function: &CudaFunction,
    query_tokens: &CudaSlice<f32>,
    query_norms: &CudaSlice<f32>,
    query_offsets: &CudaSlice<u32>,
    doc_tokens: &CudaSlice<f32>,
    doc_norms: &CudaSlice<f32>,
    row_offsets: &CudaSlice<u32>,
    candidate_mask: &CudaSlice<u8>,
    row_count: usize,
    token_dim: usize,
    query_rows: usize,
    row_scores: &mut CudaSlice<f32>,
) -> Result<()> {
    let row_count_i32 = to_i32(row_count, "batch row count")?;
    let token_dim_i32 = to_i32(token_dim, "batch token dim")?;
    let query_rows_i32 = to_i32(query_rows, "batch query rows")?;
    let mut launch = stream.launch_builder(function);
    unsafe {
        launch
            .arg(query_tokens)
            .arg(query_norms)
            .arg(query_offsets)
            .arg(&query_rows_i32)
            .arg(doc_tokens)
            .arg(doc_norms)
            .arg(row_offsets)
            .arg(candidate_mask)
            .arg(&row_count_i32)
            .arg(&token_dim_i32)
            .arg(row_scores)
            .launch(batch_score_config(row_count, query_rows)?)
    }
    .map(|_| ())
    .map_err(cuda_error("batch score launch"))
}

#[cfg(sextant_cuvs)]
#[allow(clippy::too_many_arguments)]
fn launch_topk(
    stream: &std::sync::Arc<CudaStream>,
    function: &CudaFunction,
    row_scores: &CudaSlice<f32>,
    id_hi: &CudaSlice<u64>,
    id_lo: &CudaSlice<u64>,
    candidate_mask: &CudaSlice<u8>,
    row_count: usize,
    k: usize,
    out_hi: &mut CudaSlice<u64>,
    out_lo: &mut CudaSlice<u64>,
    out_scores: &mut CudaSlice<f32>,
    out_count: &mut CudaSlice<u32>,
) -> Result<()> {
    let row_count_i32 = to_i32(row_count, "row count")?;
    let k_i32 = to_i32(k, "k")?;
    let mut launch = stream.launch_builder(function);
    unsafe {
        launch
            .arg(row_scores)
            .arg(id_hi)
            .arg(id_lo)
            .arg(candidate_mask)
            .arg(&row_count_i32)
            .arg(&k_i32)
            .arg(out_hi)
            .arg(out_lo)
            .arg(out_scores)
            .arg(out_count)
            .launch(single_thread_config())
    }
    .map(|_| ())
    .map_err(cuda_error("topk launch"))
}

#[cfg(sextant_cuvs)]
#[allow(clippy::too_many_arguments)]
fn launch_topk_batch(
    stream: &std::sync::Arc<CudaStream>,
    function: &CudaFunction,
    row_scores: &CudaSlice<f32>,
    id_hi: &CudaSlice<u64>,
    id_lo: &CudaSlice<u64>,
    candidate_mask: &CudaSlice<u8>,
    row_count: usize,
    query_rows: usize,
    k: usize,
    out_hi: &mut CudaSlice<u64>,
    out_lo: &mut CudaSlice<u64>,
    out_scores: &mut CudaSlice<f32>,
    out_count: &mut CudaSlice<u32>,
) -> Result<()> {
    let row_count_i32 = to_i32(row_count, "batch top-k row count")?;
    let query_rows_i32 = to_i32(query_rows, "batch top-k query rows")?;
    let k_i32 = to_i32(k, "batch top-k k")?;
    let mut launch = stream.launch_builder(function);
    unsafe {
        launch
            .arg(row_scores)
            .arg(id_hi)
            .arg(id_lo)
            .arg(candidate_mask)
            .arg(&row_count_i32)
            .arg(&query_rows_i32)
            .arg(&k_i32)
            .arg(out_hi)
            .arg(out_lo)
            .arg(out_scores)
            .arg(out_count)
            .launch(query_grid_config(query_rows)?)
    }
    .map(|_| ())
    .map_err(cuda_error("batch top-k launch"))
}

#[cfg(sextant_cuvs)]
#[allow(clippy::too_many_arguments)]
fn launch_merge(
    stream: &std::sync::Arc<CudaStream>,
    function: &CudaFunction,
    chunk_hi: &CudaSlice<u64>,
    chunk_lo: &CudaSlice<u64>,
    chunk_scores: &CudaSlice<f32>,
    chunk_count: &CudaSlice<u32>,
    global_hi: &mut CudaSlice<u64>,
    global_lo: &mut CudaSlice<u64>,
    global_scores: &mut CudaSlice<f32>,
    global_count: &mut CudaSlice<u32>,
    k: usize,
) -> Result<()> {
    if k > MAXSIM_CUDA_MAX_K {
        return Err(invalid("MaxSim CUDA k exceeds kernel max"));
    }
    let k_i32 = to_i32(k, "k")?;
    let mut launch = stream.launch_builder(function);
    unsafe {
        launch
            .arg(chunk_hi)
            .arg(chunk_lo)
            .arg(chunk_scores)
            .arg(chunk_count)
            .arg(global_hi)
            .arg(global_lo)
            .arg(global_scores)
            .arg(global_count)
            .arg(&k_i32)
            .launch(single_thread_config())
    }
    .map(|_| ())
    .map_err(cuda_error("merge launch"))
}

#[cfg(sextant_cuvs)]
#[allow(clippy::too_many_arguments)]
fn launch_merge_batch(
    stream: &std::sync::Arc<CudaStream>,
    function: &CudaFunction,
    chunk_hi: &CudaSlice<u64>,
    chunk_lo: &CudaSlice<u64>,
    chunk_scores: &CudaSlice<f32>,
    chunk_count: &CudaSlice<u32>,
    global_hi: &mut CudaSlice<u64>,
    global_lo: &mut CudaSlice<u64>,
    global_scores: &mut CudaSlice<f32>,
    global_count: &mut CudaSlice<u32>,
    query_rows: usize,
    k: usize,
) -> Result<()> {
    if k > MAXSIM_CUDA_MAX_K {
        return Err(invalid("batched MaxSim CUDA k exceeds kernel max"));
    }
    let query_rows_i32 = to_i32(query_rows, "batch merge query rows")?;
    let k_i32 = to_i32(k, "batch merge k")?;
    let mut launch = stream.launch_builder(function);
    unsafe {
        launch
            .arg(chunk_hi)
            .arg(chunk_lo)
            .arg(chunk_scores)
            .arg(chunk_count)
            .arg(global_hi)
            .arg(global_lo)
            .arg(global_scores)
            .arg(global_count)
            .arg(&query_rows_i32)
            .arg(&k_i32)
            .launch(query_grid_config(query_rows)?)
    }
    .map(|_| ())
    .map_err(cuda_error("batch merge launch"))
}

#[cfg(sextant_cuvs)]
fn alloc_device<T>(
    stream: &std::sync::Arc<CudaStream>,
    len: usize,
    name: &'static str,
) -> Result<CudaSlice<T>>
where
    T: cudarc::driver::DeviceRepr + ValidAsZeroBits,
{
    stream.alloc_zeros(len).map_err(cuda_error(name))
}

#[cfg(sextant_cuvs)]
fn to_i32(value: usize, name: &'static str) -> Result<i32> {
    i32::try_from(value).map_err(|_| invalid(format!("MaxSim CUDA {name} exceeds i32")))
}

#[cfg(sextant_cuvs)]
fn row_block_config(rows: usize) -> Result<LaunchConfig> {
    Ok(LaunchConfig {
        grid_dim: (
            u32::try_from(rows).map_err(|_| invalid("MaxSim CUDA grid exceeds u32"))?,
            1,
            1,
        ),
        block_dim: (THREADS as u32, 1, 1),
        shared_mem_bytes: 0,
    })
}

#[cfg(sextant_cuvs)]
fn batch_score_config(rows: usize, query_rows: usize) -> Result<LaunchConfig> {
    Ok(LaunchConfig {
        grid_dim: (
            u32::try_from(rows).map_err(|_| invalid("batched MaxSim CUDA row grid exceeds u32"))?,
            u32::try_from(query_rows)
                .map_err(|_| invalid("batched MaxSim CUDA query grid exceeds u32"))?,
            1,
        ),
        block_dim: (THREADS as u32, 1, 1),
        shared_mem_bytes: 0,
    })
}

#[cfg(sextant_cuvs)]
fn query_grid_config(query_rows: usize) -> Result<LaunchConfig> {
    Ok(LaunchConfig {
        grid_dim: (
            u32::try_from(query_rows)
                .map_err(|_| invalid("batched MaxSim CUDA query grid exceeds u32"))?,
            1,
            1,
        ),
        block_dim: (1, 1, 1),
        shared_mem_bytes: 0,
    })
}

#[cfg(sextant_cuvs)]
fn single_thread_config() -> LaunchConfig {
    LaunchConfig {
        grid_dim: (1, 1, 1),
        block_dim: (1, 1, 1),
        shared_mem_bytes: 0,
    }
}

#[cfg(not(sextant_cuvs))]
pub(super) fn run<F>(_request: MaxSimCudaRequest<'_>, _load_chunk: &mut F) -> Result<MaxSimCudaTopK>
where
    F: FnMut(usize, usize, usize) -> Result<Option<MaxSimCudaChunk>>,
{
    Err(crate::error::sextant_error(
        crate::error::CALYX_SEXTANT_GPU_PARITY_UNAVAILABLE,
        crate::cuvs_unavailable_reason("MaxSim CUDA search"),
    ))
}

#[cfg(not(sextant_cuvs))]
pub(super) fn run_batch<F>(
    _request: MaxSimCudaBatchRequest<'_>,
    _load_chunk: &mut F,
) -> Result<MaxSimCudaBatchTopK>
where
    F: FnMut(usize, usize, usize) -> Result<Option<MaxSimCudaChunk>>,
{
    Err(crate::error::sextant_error(
        crate::error::CALYX_SEXTANT_GPU_PARITY_UNAVAILABLE,
        crate::cuvs_unavailable_reason("batched MaxSim CUDA search"),
    ))
}
