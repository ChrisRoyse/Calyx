use cudarc::driver::CudaSlice;

use crate::cpu::{check_finite, check_shape_2d};
use crate::cuda::distance::{launch_cosine_batch_gpu, read_checked_device_output};
use crate::{BlockId, CudaContext, ForgeError, Result};

const RESIDENT_REMEDIATION: &str =
    "Upload finite candidate blocks once, then score resident candidates with finite queries";

pub struct DeviceCandidateBlock {
    block_id: BlockId,
    dim: usize,
    n_cands: usize,
    values: CudaSlice<f32>,
}

impl DeviceCandidateBlock {
    pub fn block_id(&self) -> BlockId {
        self.block_id
    }

    pub fn dim(&self) -> usize {
        self.dim
    }

    pub fn n_cands(&self) -> usize {
        self.n_cands
    }
}

pub fn upload_candidate_block(
    ctx: &CudaContext,
    block_id: BlockId,
    candidates: &[f32],
    dim: usize,
) -> Result<DeviceCandidateBlock> {
    let n_cands = validate_candidates(candidates, dim)?;
    let values = ctx
        .inner()
        .default_stream()
        .clone_htod(candidates)
        .map_err(|err| {
            device_unavailable(ctx, format!("resident candidate upload failed: {err}"))
        })?;
    Ok(DeviceCandidateBlock {
        block_id,
        dim,
        n_cands,
        values,
    })
}

pub fn cosine_resident_host(
    ctx: &CudaContext,
    query: &[f32],
    block: &DeviceCandidateBlock,
    out: &mut [f32],
) -> Result<()> {
    validate_query(query, block.dim)?;
    check_shape_2d(out, block.n_cands, 1, "cuda resident cosine output")?;
    out.fill(0.0);
    if block.n_cands == 0 {
        return Ok(());
    }
    let stream = ctx.inner().default_stream();
    let query_dev = stream.clone_htod(query).map_err(|err| {
        device_unavailable(ctx, format!("resident cosine query upload failed: {err}"))
    })?;
    let mut out_dev = stream.alloc_zeros(out.len()).map_err(|err| {
        device_unavailable(
            ctx,
            format!("resident cosine output allocation failed: {err}"),
        )
    })?;
    launch_cosine_batch_gpu(
        ctx,
        &query_dev,
        &block.values,
        block.dim,
        block.n_cands,
        &mut out_dev,
    )?;
    let values = read_checked_device_output(ctx, "cosine_batch_gpu", &out_dev, true)?;
    out.copy_from_slice(&values);
    Ok(())
}

fn validate_candidates(candidates: &[f32], dim: usize) -> Result<usize> {
    if dim == 0 {
        return Err(resident_shape_error(
            "candidate dim must be non-zero for resident cosine blocks",
        ));
    }
    if !candidates.len().is_multiple_of(dim) {
        return Err(resident_shape_error(
            "candidate length must be an integer number of rows",
        ));
    }
    let n_cands = candidates.len() / dim;
    check_shape_2d(candidates, n_cands, dim, "cuda resident candidates")?;
    check_finite(candidates, "cuda resident candidates")?;
    Ok(n_cands)
}

fn validate_query(query: &[f32], dim: usize) -> Result<()> {
    check_shape_2d(query, 1, dim, "cuda resident cosine query")?;
    check_finite(query, "cuda resident cosine query")
}

fn resident_shape_error(detail: &str) -> ForgeError {
    ForgeError::ShapeMismatch {
        expected: vec![1],
        got: vec![0],
        remediation: detail.to_string(),
    }
}

fn device_unavailable(ctx: &CudaContext, detail: String) -> ForgeError {
    ForgeError::DeviceUnavailable {
        device: format!("cuda:{}", ctx.device_idx()),
        detail,
        remediation: RESIDENT_REMEDIATION.to_string(),
    }
}
