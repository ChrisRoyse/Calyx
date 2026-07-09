use std::cmp::Ordering;
use std::collections::BinaryHeap;
use std::str;
use std::sync::Arc;

use cudarc::driver::{CudaModule, CudaSlice, LaunchConfig, PushKernelArg};
use cudarc::nvrtc::Ptx;

use crate::cuda::kernels::{TOPK_CUBIN, TOPK_PTX};
use crate::{CUDA_EXACT_TOPK_MAX_K, CudaContext, ForgeError, Result};

const TOPK_BLOCK: usize = CUDA_EXACT_TOPK_MAX_K;
const TOPK_REMEDIATION: &str =
    "Reject non-finite scores and keep deterministic score/index ordering";
const DEVICE_REMEDIATION: &str = "Check CUDA, embedded topk PTX, and CUDA GPU device availability";

pub fn topk_gpu(
    ctx: &CudaContext,
    scores: &CudaSlice<f32>,
    k: usize,
    n: usize,
) -> Result<Vec<(usize, f32)>> {
    check_device_len(scores.len(), n)?;
    if k == 0 || n == 0 {
        return Ok(Vec::new());
    }
    let k_eff = k.min(n);
    if k_eff > TOPK_BLOCK {
        return Err(ForgeError::ShapeMismatch {
            expected: vec![TOPK_BLOCK],
            got: vec![k_eff],
            remediation: format!(
                "cuda topk is exact only for global k <= {CUDA_EXACT_TOPK_MAX_K}; use CPU topk or add a multi-pass exact CUDA merge"
            ),
        });
    }
    let chunk_k = k_eff;

    let chunks = n.div_ceil(TOPK_BLOCK);
    let out_len = chunks
        .checked_mul(chunk_k)
        .ok_or_else(|| ForgeError::ShapeMismatch {
            expected: vec![chunks, chunk_k],
            got: vec![usize::MAX],
            remediation: "cuda topk output shape overflows usize".to_string(),
        })?;
    let stream = ctx.inner().default_stream();
    let mut out_indices = stream
        .alloc_zeros(out_len)
        .map_err(|err| device_unavailable(ctx, format!("topk index allocation failed: {err}")))?;
    let mut out_scores = stream
        .alloc_zeros(out_len)
        .map_err(|err| device_unavailable(ctx, format!("topk score allocation failed: {err}")))?;

    launch_topk(
        ctx,
        scores,
        n,
        chunk_k,
        chunks,
        &mut out_indices,
        &mut out_scores,
    )?;
    let indices = stream
        .clone_dtoh(&out_indices)
        .map_err(|err| device_unavailable(ctx, format!("topk index readback failed: {err}")))?;
    let values = stream
        .clone_dtoh(&out_scores)
        .map_err(|err| device_unavailable(ctx, format!("topk score readback failed: {err}")))?;
    merge_chunks(ctx, &indices, &values, n, k_eff, chunk_k)
}

pub fn topk_host(ctx: &CudaContext, scores: &[f32], k: usize) -> Result<Vec<(usize, f32)>> {
    if k == 0 || scores.is_empty() {
        return Ok(Vec::new());
    }
    let stream = ctx.inner().default_stream();
    let scores_dev = stream
        .clone_htod(scores)
        .map_err(|err| device_unavailable(ctx, format!("topk scores copy failed: {err}")))?;
    topk_gpu(ctx, &scores_dev, k, scores.len())
}

fn launch_topk(
    ctx: &CudaContext,
    scores: &CudaSlice<f32>,
    n: usize,
    chunk_k: usize,
    chunks: usize,
    out_indices: &mut CudaSlice<i32>,
    out_scores: &mut CudaSlice<f32>,
) -> Result<()> {
    let n_i32 = to_i32(n, "count")?;
    let k_i32 = to_i32(chunk_k, "k")?;
    let chunks_u32 = u32::try_from(chunks).map_err(|_| ForgeError::ShapeMismatch {
        expected: vec![u32::MAX as usize],
        got: vec![chunks],
        remediation: "cuda topk chunk count exceeds grid dimension limit".to_string(),
    })?;
    let module = topk_module(ctx)?;
    let func = ctx
        .cached_function(&module, "topk.bitonic_topk_f32", "bitonic_topk_f32")
        .map_err(|err| device_unavailable(ctx, format!("topk load function failed: {err}")))?;
    let stream = ctx.inner().default_stream();
    let cfg = LaunchConfig {
        grid_dim: (chunks_u32, 1, 1),
        block_dim: (TOPK_BLOCK as u32, 1, 1),
        shared_mem_bytes: 0,
    };
    let mut launch = stream.launch_builder(func.as_ref());
    unsafe {
        launch
            .arg(scores)
            .arg(&n_i32)
            .arg(&k_i32)
            .arg(out_indices)
            .arg(out_scores)
            .launch(cfg)
    }
    .map_err(|err| device_unavailable(ctx, format!("topk kernel launch failed: {err}")))?;
    stream
        .synchronize()
        .map_err(|err| device_unavailable(ctx, format!("topk stream sync failed: {err}")))?;
    Ok(())
}

fn merge_chunks(
    ctx: &CudaContext,
    indices: &[i32],
    scores: &[f32],
    n: usize,
    k_eff: usize,
    chunk_k: usize,
) -> Result<Vec<(usize, f32)>> {
    let chunks = n.div_ceil(TOPK_BLOCK);
    let inputs = ChunkMergeInputs {
        ctx,
        indices,
        scores,
        n,
        chunk_k,
    };
    let mut heap = BinaryHeap::with_capacity(chunks.min(k_eff));
    for chunk in 0..chunks {
        push_chunk_entry(&inputs, &mut heap, chunk, 0)?;
    }

    let mut pairs = Vec::with_capacity(k_eff);
    while pairs.len() < k_eff {
        let Some(entry) = heap.pop() else {
            break;
        };
        pairs.push((entry.index, entry.score));
        push_chunk_entry(&inputs, &mut heap, entry.chunk, entry.offset + 1)?;
    }
    Ok(pairs)
}

struct ChunkMergeInputs<'a> {
    ctx: &'a CudaContext,
    indices: &'a [i32],
    scores: &'a [f32],
    n: usize,
    chunk_k: usize,
}

#[derive(Clone, Copy, Debug)]
struct HeapEntry {
    score: f32,
    index: usize,
    chunk: usize,
    offset: usize,
}

impl PartialEq for HeapEntry {
    fn eq(&self, other: &Self) -> bool {
        self.score.to_bits() == other.score.to_bits()
            && self.index == other.index
            && self.chunk == other.chunk
            && self.offset == other.offset
    }
}

impl Eq for HeapEntry {}

impl PartialOrd for HeapEntry {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for HeapEntry {
    fn cmp(&self, other: &Self) -> Ordering {
        self.score
            .total_cmp(&other.score)
            .then_with(|| other.index.cmp(&self.index))
            .then_with(|| other.chunk.cmp(&self.chunk))
            .then_with(|| other.offset.cmp(&self.offset))
    }
}

fn push_chunk_entry(
    inputs: &ChunkMergeInputs<'_>,
    heap: &mut BinaryHeap<HeapEntry>,
    chunk: usize,
    offset: usize,
) -> Result<()> {
    let chunk_len = (inputs.n - chunk * TOPK_BLOCK).min(TOPK_BLOCK);
    if offset >= chunk_len.min(inputs.chunk_k) {
        return Ok(());
    }
    let pos = chunk * inputs.chunk_k + offset;
    let index = inputs.indices[pos];
    let score = inputs.scores[pos];
    if index < 0 {
        return Err(numerical(
            "topk_gpu",
            "NaN score sentinel returned by kernel".to_string(),
        ));
    }
    if !score.is_finite() {
        return Err(numerical(
            "topk_gpu",
            format!("non-finite score at output {pos}: {score}"),
        ));
    }
    let index = index as usize;
    if index >= inputs.n {
        return Err(device_unavailable(
            inputs.ctx,
            format!("topk kernel returned out-of-range index {index}"),
        ));
    }
    heap.push(HeapEntry {
        score,
        index,
        chunk,
        offset,
    });
    Ok(())
}

fn topk_module(ctx: &CudaContext) -> Result<Arc<CudaModule>> {
    if let Some(module) = ctx.topk_module_cache().get() {
        return Ok(module.clone());
    }
    match ctx
        .inner()
        .load_module(Ptx::from_binary(TOPK_CUBIN.to_vec()))
    {
        Ok(module) => {
            let _ = ctx.topk_module_cache().set(module.clone());
            Ok(module)
        }
        Err(cubin_err) => {
            let module = topk_ptx_module(ctx, cubin_err)?;
            let _ = ctx.topk_module_cache().set(module.clone());
            Ok(module)
        }
    }
}

fn topk_ptx_module(
    ctx: &CudaContext,
    cubin_err: cudarc::driver::DriverError,
) -> Result<Arc<CudaModule>> {
    let ptx = str::from_utf8(TOPK_PTX)
        .map_err(|err| device_unavailable(ctx, format!("topk PTX is not UTF-8: {err}")))?;
    ctx.inner()
        .load_module(Ptx::from_src(ptx))
        .map_err(|ptx_err| {
            device_unavailable(
                ctx,
                format!("topk CUBIN load failed: {cubin_err}; PTX fallback load failed: {ptx_err}"),
            )
        })
}

fn check_device_len(actual: usize, expected: usize) -> Result<()> {
    if actual == expected {
        return Ok(());
    }
    Err(ForgeError::ShapeMismatch {
        expected: vec![expected],
        got: vec![actual],
        remediation: "cuda topk scores length must equal n".to_string(),
    })
}

fn to_i32(value: usize, name: &str) -> Result<i32> {
    i32::try_from(value).map_err(|_| ForgeError::ShapeMismatch {
        expected: vec![i32::MAX as usize],
        got: vec![value],
        remediation: format!("cuda topk {name} exceeds i32 kernel argument limit"),
    })
}

fn numerical(op: &'static str, detail: String) -> ForgeError {
    ForgeError::NumericalInvariant {
        op: op.to_string(),
        detail,
        remediation: TOPK_REMEDIATION.to_string(),
    }
}

fn device_unavailable(ctx: &CudaContext, detail: String) -> ForgeError {
    ForgeError::DeviceUnavailable {
        device: format!("cuda:{}", ctx.device_idx()),
        detail,
        remediation: DEVICE_REMEDIATION.to_string(),
    }
}
