use std::str;
use std::sync::Arc;

use cudarc::driver::{CudaModule, CudaSlice, LaunchConfig, PushKernelArg};
use cudarc::nvrtc::Ptx;

use crate::cpu::{check_finite, check_shape_2d};
use crate::cuda::kernels::DISTANCE_PTX;
use crate::{CudaContext, ForgeError, Result};

const BLOCK_THREADS: u32 = 256;
const DISTANCE_REMEDIATION: &str =
    "Check CUDA distance kernel inputs and fail closed instead of returning invalid scores";
const DEVICE_REMEDIATION: &str =
    "Check CUDA, embedded distance PTX, and CUDA GPU device availability";

pub fn cosine_batch_gpu(
    ctx: &CudaContext,
    query: &CudaSlice<f32>,
    candidates: &CudaSlice<f32>,
    dim: usize,
    n_cands: usize,
    out: &mut CudaSlice<f32>,
) -> Result<()> {
    launch_distance(
        ctx,
        "cosine_batch_gpu",
        "cosine_batch_f32",
        query,
        candidates,
        dim,
        n_cands,
        out,
    )?;
    check_device_output(ctx, "cosine_batch_gpu", out, true)
}

pub fn dot_batch_gpu(
    ctx: &CudaContext,
    query: &CudaSlice<f32>,
    candidates: &CudaSlice<f32>,
    dim: usize,
    n_cands: usize,
    out: &mut CudaSlice<f32>,
) -> Result<()> {
    launch_distance(
        ctx,
        "dot_batch_gpu",
        "dot_batch_f32",
        query,
        candidates,
        dim,
        n_cands,
        out,
    )?;
    check_device_output(ctx, "dot_batch_gpu", out, false)
}

pub fn l2_batch_gpu(
    ctx: &CudaContext,
    query: &CudaSlice<f32>,
    candidates: &CudaSlice<f32>,
    dim: usize,
    n_cands: usize,
    out: &mut CudaSlice<f32>,
) -> Result<()> {
    launch_distance(
        ctx,
        "l2_batch_gpu",
        "l2_batch_f32",
        query,
        candidates,
        dim,
        n_cands,
        out,
    )?;
    check_device_output(ctx, "l2_batch_gpu", out, false)
}

pub fn cosine_host(
    ctx: &CudaContext,
    query: &[f32],
    candidates: &[f32],
    dim: usize,
    out: &mut [f32],
) -> Result<()> {
    distance_host(
        ctx,
        "cosine_batch_gpu",
        query,
        candidates,
        dim,
        out,
        cosine_batch_gpu,
    )
}

pub fn dot_host(
    ctx: &CudaContext,
    query: &[f32],
    candidates: &[f32],
    dim: usize,
    out: &mut [f32],
) -> Result<()> {
    distance_host(
        ctx,
        "dot_batch_gpu",
        query,
        candidates,
        dim,
        out,
        dot_batch_gpu,
    )
}

pub fn l2_host(
    ctx: &CudaContext,
    query: &[f32],
    candidates: &[f32],
    dim: usize,
    out: &mut [f32],
) -> Result<()> {
    distance_host(
        ctx,
        "l2_batch_gpu",
        query,
        candidates,
        dim,
        out,
        l2_batch_gpu,
    )
}

pub fn normalize_rows_gpu(
    ctx: &CudaContext,
    vecs: &mut CudaSlice<f32>,
    rows: usize,
    dim: usize,
) -> Result<()> {
    check_device_shape(vecs.len(), rows, dim, "cuda normalize input")?;
    if rows == 0 {
        return Ok(());
    }
    launch_normalize(ctx, vecs, rows, dim)?;
    check_device_output(ctx, "normalize_rows_gpu", vecs, false)
}

pub fn normalize_host(ctx: &CudaContext, vecs: &mut [f32], dim: usize) -> Result<()> {
    let rows = validate_normalize_host_inputs(vecs, dim)?;
    if vecs.is_empty() {
        return Ok(());
    }

    let stream = ctx.inner().default_stream();
    let mut vecs_dev = stream
        .clone_htod(vecs)
        .map_err(|err| device_unavailable(ctx, format!("normalize input copy failed: {err}")))?;
    normalize_rows_gpu(ctx, &mut vecs_dev, rows, dim)?;
    let result = stream
        .clone_dtoh(&vecs_dev)
        .map_err(|err| device_unavailable(ctx, format!("normalize output copy failed: {err}")))?;
    check_finite(&result, "cuda normalize")?;
    vecs.copy_from_slice(&result);
    Ok(())
}

type DistanceKernel = fn(
    &CudaContext,
    &CudaSlice<f32>,
    &CudaSlice<f32>,
    usize,
    usize,
    &mut CudaSlice<f32>,
) -> Result<()>;

fn distance_host(
    ctx: &CudaContext,
    op: &'static str,
    query: &[f32],
    candidates: &[f32],
    dim: usize,
    out: &mut [f32],
    kernel: DistanceKernel,
) -> Result<()> {
    validate_host_inputs(op, query, candidates, dim, out)?;
    out.fill(0.0);
    if out.is_empty() {
        return Ok(());
    }

    let stream = ctx.inner().default_stream();
    let query_dev = stream
        .clone_htod(query)
        .map_err(|err| device_unavailable(ctx, format!("{op} query copy failed: {err}")))?;
    let candidates_dev = stream
        .clone_htod(candidates)
        .map_err(|err| device_unavailable(ctx, format!("{op} candidates copy failed: {err}")))?;
    let mut out_dev = stream
        .alloc_zeros(out.len())
        .map_err(|err| device_unavailable(ctx, format!("{op} output allocation failed: {err}")))?;

    kernel(
        ctx,
        &query_dev,
        &candidates_dev,
        dim,
        out.len(),
        &mut out_dev,
    )?;
    let result = stream
        .clone_dtoh(&out_dev)
        .map_err(|err| device_unavailable(ctx, format!("{op} output copy failed: {err}")))?;
    out.copy_from_slice(&result);
    check_finite(out, op)?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn launch_distance(
    ctx: &CudaContext,
    op: &'static str,
    kernel_name: &'static str,
    query: &CudaSlice<f32>,
    candidates: &CudaSlice<f32>,
    dim: usize,
    n_cands: usize,
    out: &mut CudaSlice<f32>,
) -> Result<()> {
    check_device_shape(query.len(), 1, dim, "cuda distance query")?;
    check_device_shape(candidates.len(), n_cands, dim, "cuda distance candidates")?;
    check_device_shape(out.len(), n_cands, 1, "cuda distance output")?;
    if n_cands == 0 {
        return Ok(());
    }

    let dim_i32 = to_i32(dim, "dim")?;
    let n_cands_i32 = to_i32(n_cands, "n_cands")?;
    let n_cands_u32 = u32::try_from(n_cands).map_err(|_| ForgeError::ShapeMismatch {
        expected: vec![u32::MAX as usize],
        got: vec![n_cands],
        remediation: "cuda distance n_cands exceeds grid dimension limit".to_string(),
    })?;
    let module = distance_module(ctx)?;
    let func = module
        .load_function(kernel_name)
        .map_err(|err| device_unavailable(ctx, format!("{op} load function failed: {err}")))?;
    let stream = ctx.inner().default_stream();
    let cfg = LaunchConfig {
        grid_dim: (n_cands_u32, 1, 1),
        block_dim: (BLOCK_THREADS, 1, 1),
        shared_mem_bytes: 0,
    };

    let mut launch = stream.launch_builder(&func);
    unsafe {
        launch
            .arg(query)
            .arg(candidates)
            .arg(&dim_i32)
            .arg(&n_cands_i32)
            .arg(out)
            .launch(cfg)
    }
    .map_err(|err| device_unavailable(ctx, format!("{op} kernel launch failed: {err}")))?;
    stream
        .synchronize()
        .map_err(|err| device_unavailable(ctx, format!("{op} stream sync failed: {err}")))?;
    Ok(())
}

fn launch_normalize(
    ctx: &CudaContext,
    vecs: &mut CudaSlice<f32>,
    rows: usize,
    dim: usize,
) -> Result<()> {
    let rows_i32 = to_i32(rows, "rows")?;
    let rows_u32 = u32::try_from(rows).map_err(|_| ForgeError::ShapeMismatch {
        expected: vec![u32::MAX as usize],
        got: vec![rows],
        remediation: "cuda normalize row count exceeds grid dimension limit".to_string(),
    })?;
    let dim_i32 = to_i32(dim, "dim")?;
    let module = distance_module(ctx)?;
    let func = module
        .load_function("normalize_rows_f32")
        .map_err(|err| device_unavailable(ctx, format!("normalize load function failed: {err}")))?;
    let stream = ctx.inner().default_stream();
    let cfg = LaunchConfig {
        grid_dim: (rows_u32, 1, 1),
        block_dim: (BLOCK_THREADS, 1, 1),
        shared_mem_bytes: 0,
    };

    let mut launch = stream.launch_builder(&func);
    unsafe { launch.arg(vecs).arg(&dim_i32).arg(&rows_i32).launch(cfg) }
        .map_err(|err| device_unavailable(ctx, format!("normalize kernel launch failed: {err}")))?;
    stream
        .synchronize()
        .map_err(|err| device_unavailable(ctx, format!("normalize stream sync failed: {err}")))?;
    Ok(())
}

fn distance_module(ctx: &CudaContext) -> Result<Arc<CudaModule>> {
    if let Some(module) = ctx.distance_module_cache().get() {
        return Ok(module.clone());
    }
    let ptx = str::from_utf8(DISTANCE_PTX)
        .map_err(|err| device_unavailable(ctx, format!("distance PTX is not UTF-8: {err}")))?;
    let module = ctx
        .inner()
        .load_module(Ptx::from_src(ptx))
        .map_err(|err| device_unavailable(ctx, format!("distance PTX load failed: {err}")))?;
    let _ = ctx.distance_module_cache().set(module.clone());
    Ok(module)
}

fn check_device_output(
    ctx: &CudaContext,
    op: &'static str,
    out: &CudaSlice<f32>,
    sentinel: bool,
) -> Result<()> {
    let values = ctx
        .inner()
        .default_stream()
        .clone_dtoh(out)
        .map_err(|err| device_unavailable(ctx, format!("{op} output readback failed: {err}")))?;
    for (idx, value) in values.iter().enumerate() {
        if sentinel && *value <= -1.5 {
            return Err(numerical(
                op,
                format!("zero-norm query or candidate at index {idx}"),
            ));
        }
        if !value.is_finite() {
            return Err(numerical(
                op,
                format!("non-finite output at index {idx}: {value}"),
            ));
        }
    }
    Ok(())
}

fn validate_host_inputs(
    op: &'static str,
    query: &[f32],
    candidates: &[f32],
    dim: usize,
    out: &[f32],
) -> Result<()> {
    check_shape_2d(query, 1, dim, "cuda distance query")?;
    check_shape_2d(candidates, out.len(), dim, "cuda distance candidates")?;
    check_finite(query, op)?;
    check_finite(candidates, op)?;
    Ok(())
}

fn validate_normalize_host_inputs(vecs: &[f32], dim: usize) -> Result<usize> {
    if dim == 0 {
        if vecs.is_empty() {
            return Ok(0);
        }
        return Err(ForgeError::ShapeMismatch {
            expected: vec![0],
            got: vec![vecs.len()],
            remediation: "dim=0 is valid only for an empty matrix".to_string(),
        });
    }
    if !vecs.len().is_multiple_of(dim) {
        return Err(ForgeError::ShapeMismatch {
            expected: vec![dim],
            got: vec![vecs.len()],
            remediation: "normalize input length must be an integer number of rows".to_string(),
        });
    }
    let rows = vecs.len() / dim;
    check_shape_2d(vecs, rows, dim, "cuda normalize input")?;
    check_finite(vecs, "cuda normalize")?;
    Ok(rows)
}

fn check_device_shape(len: usize, rows: usize, cols: usize, name: &str) -> Result<()> {
    let expected_len = rows
        .checked_mul(cols)
        .ok_or_else(|| ForgeError::ShapeMismatch {
            expected: vec![rows, cols],
            got: vec![len],
            remediation: format!("{name} shape overflows usize"),
        })?;
    if len == expected_len {
        return Ok(());
    }
    Err(ForgeError::ShapeMismatch {
        expected: vec![rows, cols],
        got: vec![len],
        remediation: format!("{name} length does not match rows*cols"),
    })
}

fn to_i32(value: usize, name: &str) -> Result<i32> {
    i32::try_from(value).map_err(|_| ForgeError::ShapeMismatch {
        expected: vec![i32::MAX as usize],
        got: vec![value],
        remediation: format!("cuda distance {name} exceeds i32 kernel argument limit"),
    })
}

fn numerical(op: &'static str, detail: String) -> ForgeError {
    ForgeError::NumericalInvariant {
        op: op.to_string(),
        detail,
        remediation: DISTANCE_REMEDIATION.to_string(),
    }
}

fn device_unavailable(ctx: &CudaContext, detail: String) -> ForgeError {
    ForgeError::DeviceUnavailable {
        device: format!("cuda:{}", ctx.device_idx()),
        detail,
        remediation: DEVICE_REMEDIATION.to_string(),
    }
}
