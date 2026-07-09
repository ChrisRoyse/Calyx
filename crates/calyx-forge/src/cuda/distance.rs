use std::str;
use std::sync::Arc;

use cudarc::driver::{CudaModule, CudaSlice, LaunchConfig, PushKernelArg};
use cudarc::nvrtc::Ptx;

use crate::cpu::{check_finite, check_shape_2d};
use crate::cuda::kernels::{DISTANCE_CUBIN, DISTANCE_PTX};
use crate::cuda::validate::{check_device_f32, read_checked_device_f32};
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

pub(crate) fn launch_cosine_batch_gpu(
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
    )
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
        ("cosine_batch_gpu", "cosine_batch_f32", true),
        query,
        candidates,
        dim,
        out,
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
        ("dot_batch_gpu", "dot_batch_f32", false),
        query,
        candidates,
        dim,
        out,
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
        ("l2_batch_gpu", "l2_batch_f32", false),
        query,
        candidates,
        dim,
        out,
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
    launch_normalize(ctx, &mut vecs_dev, rows, dim)?;
    let result = read_checked_device_output(ctx, "normalize_rows_gpu", &vecs_dev, false)?;
    vecs.copy_from_slice(&result);
    Ok(())
}

fn distance_host(
    ctx: &CudaContext,
    kernel: (&'static str, &'static str, bool),
    query: &[f32],
    candidates: &[f32],
    dim: usize,
    out: &mut [f32],
) -> Result<()> {
    let (op, kernel_name, sentinel) = kernel;
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

    launch_distance(
        ctx,
        op,
        kernel_name,
        &query_dev,
        &candidates_dev,
        dim,
        out.len(),
        &mut out_dev,
    )?;
    let result = read_checked_device_output(ctx, op, &out_dev, sentinel)?;
    out.copy_from_slice(&result);
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
    let func = ctx
        .cached_function(&module, distance_cache_key(kernel_name), kernel_name)
        .map_err(|err| device_unavailable(ctx, format!("{op} load function failed: {err}")))?;
    let stream = ctx.inner().default_stream();
    let cfg = LaunchConfig {
        grid_dim: (n_cands_u32, 1, 1),
        block_dim: (BLOCK_THREADS, 1, 1),
        shared_mem_bytes: 0,
    };

    let mut launch = stream.launch_builder(func.as_ref());
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
    let func = ctx
        .cached_function(&module, "distance.normalize_rows_f32", "normalize_rows_f32")
        .map_err(|err| device_unavailable(ctx, format!("normalize load function failed: {err}")))?;
    let stream = ctx.inner().default_stream();
    let cfg = LaunchConfig {
        grid_dim: (rows_u32, 1, 1),
        block_dim: (BLOCK_THREADS, 1, 1),
        shared_mem_bytes: 0,
    };

    let mut launch = stream.launch_builder(func.as_ref());
    unsafe { launch.arg(vecs).arg(&dim_i32).arg(&rows_i32).launch(cfg) }
        .map_err(|err| device_unavailable(ctx, format!("normalize kernel launch failed: {err}")))?;
    Ok(())
}

pub(crate) fn distance_module(ctx: &CudaContext) -> Result<Arc<CudaModule>> {
    if let Some(module) = ctx.distance_module_cache().get() {
        return Ok(module.clone());
    }
    match ctx
        .inner()
        .load_module(Ptx::from_binary(DISTANCE_CUBIN.to_vec()))
    {
        Ok(module) => {
            let _ = ctx.distance_module_cache().set(module.clone());
            Ok(module)
        }
        Err(cubin_err) => {
            let module = distance_ptx_module(ctx, cubin_err)?;
            let _ = ctx.distance_module_cache().set(module.clone());
            Ok(module)
        }
    }
}

fn distance_cache_key(kernel_name: &'static str) -> &'static str {
    match kernel_name {
        "cosine_batch_f32" => "distance.cosine_batch_f32",
        "dot_batch_f32" => "distance.dot_batch_f32",
        "l2_batch_f32" => "distance.l2_batch_f32",
        _ => kernel_name,
    }
}

fn distance_ptx_module(
    ctx: &CudaContext,
    cubin_err: cudarc::driver::DriverError,
) -> Result<Arc<CudaModule>> {
    let ptx = str::from_utf8(DISTANCE_PTX)
        .map_err(|err| device_unavailable(ctx, format!("distance PTX is not UTF-8: {err}")))?;
    ctx.inner()
        .load_module(Ptx::from_src(ptx))
        .map_err(|ptx_err| {
            device_unavailable(
                ctx,
                format!(
                    "distance CUBIN load failed: {cubin_err}; PTX fallback load failed: {ptx_err}"
                ),
            )
        })
}

fn check_device_output(
    ctx: &CudaContext,
    op: &'static str,
    out: &CudaSlice<f32>,
    sentinel: bool,
) -> Result<()> {
    check_device_f32(ctx, op, out, sentinel, DISTANCE_REMEDIATION)
}

pub(crate) fn read_checked_device_output(
    ctx: &CudaContext,
    op: &'static str,
    out: &CudaSlice<f32>,
    sentinel: bool,
) -> Result<Vec<f32>> {
    read_checked_device_f32(ctx, op, out, sentinel, DISTANCE_REMEDIATION)
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

fn device_unavailable(ctx: &CudaContext, detail: String) -> ForgeError {
    ForgeError::DeviceUnavailable {
        device: format!("cuda:{}", ctx.device_idx()),
        detail,
        remediation: DEVICE_REMEDIATION.to_string(),
    }
}
