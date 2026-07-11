use crate::{
    BackendKind, BestConfig, ForgeError, GemmProblem, Result, build_grouped_gemm_plan,
    cpu::check_finite,
    cuda::{
        distance::{launch_cosine_batch_gpu, read_checked_device_output},
        gemm::{GemmCublasRequest, GemmDims, gemm_cublas_with_blas, new_blas},
        grouped_gemm::{execute_grouped_gemm_bench, new_grouped_blas, validate_output},
    },
};

#[cfg(test)]
use std::sync::atomic::{AtomicUsize, Ordering};

use super::{
    BenchCudaContext, BenchResult, CUDA_REQUIRED_REMEDIATION, cuda_required, ensure_nonzero,
    matrix_len, random_values, shape_error, time_op,
};

#[cfg(test)]
static CUDA_COSINE_LAUNCH_CALLS: AtomicUsize = AtomicUsize::new(0);
#[cfg(test)]
static CUDA_SYNC_CALLS: AtomicUsize = AtomicUsize::new(0);

#[allow(clippy::too_many_arguments)]
pub(super) fn bench_cuda_gemm(
    op: &str,
    ctx: Option<&BenchCudaContext>,
    iters: u32,
    flops: f64,
    a: &[f32],
    b: &[f32],
    m: usize,
    k: usize,
    n: usize,
    out: &mut [f32],
) -> Result<BenchResult> {
    let ctx = ctx.ok_or_else(|| cuda_required(op))?;
    let stream = ctx.inner().default_stream();
    let a_dev = stream
        .clone_htod(a)
        .map_err(|err| cuda_device_error(ctx, op, format!("copy GEMM A failed: {err}")))?;
    let b_dev = stream
        .clone_htod(b)
        .map_err(|err| cuda_device_error(ctx, op, format!("copy GEMM B failed: {err}")))?;
    let mut out_dev = stream
        .alloc_zeros(out.len())
        .map_err(|err| cuda_device_error(ctx, op, format!("allocate GEMM C failed: {err}")))?;
    let blas = new_blas(ctx)?;
    let result = time_op(op, iters, flops, || {
        gemm_cublas_with_blas(GemmCublasRequest {
            ctx,
            blas: blas.as_ref(),
            a: &a_dev,
            b: &b_dev,
            dims: GemmDims::new(m, k, n),
            out: &mut out_dev,
        })?;
        sync_cuda(ctx, op)
    })?;
    let values = stream
        .clone_dtoh(&out_dev)
        .map_err(|err| cuda_device_error(ctx, op, format!("read GEMM C failed: {err}")))?;
    out.copy_from_slice(&values);
    check_finite(out, "microbench::gemm")?;
    Ok(result)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn bench_cuda_cosine(
    op: &str,
    ctx: Option<&BenchCudaContext>,
    iters: u32,
    flops: f64,
    query: &[f32],
    candidates: &[f32],
    dim: usize,
    out: &mut [f32],
) -> Result<BenchResult> {
    let ctx = ctx.ok_or_else(|| cuda_required(op))?;
    let stream = ctx.inner().default_stream();
    let query_dev = stream
        .clone_htod(query)
        .map_err(|err| cuda_device_error(ctx, op, format!("copy cosine query failed: {err}")))?;
    let candidates_dev = stream.clone_htod(candidates).map_err(|err| {
        cuda_device_error(ctx, op, format!("copy cosine candidates failed: {err}"))
    })?;
    let mut out_dev = stream.alloc_zeros(out.len()).map_err(|err| {
        cuda_device_error(ctx, op, format!("allocate cosine output failed: {err}"))
    })?;
    let result = time_op(op, iters, flops, || {
        #[cfg(test)]
        CUDA_COSINE_LAUNCH_CALLS.fetch_add(1, Ordering::SeqCst);
        launch_cosine_batch_gpu(
            ctx,
            &query_dev,
            &candidates_dev,
            dim,
            out.len(),
            &mut out_dev,
        )?;
        sync_cuda(ctx, op)
    })?;
    let values = read_checked_device_output(ctx, "cosine_batch_gpu", &out_dev, true)?;
    out.copy_from_slice(&values);
    Ok(result)
}

pub(super) fn bench_grouped_gemm(
    op: &str,
    config: &BestConfig,
    shape: &[usize],
    ctx: Option<&BenchCudaContext>,
    iters: u32,
) -> Result<BenchResult> {
    if config.backend != BackendKind::Cuda {
        return Err(cuda_required(op));
    }
    let ctx = ctx.ok_or_else(|| cuda_required(op))?;
    let (groups, m, k, n) = grouped_shape(shape)?;
    let a_len = matrix_len(
        groups,
        matrix_len(m, k, "grouped A matrix")?,
        "grouped A slab",
    )?;
    let b_len = matrix_len(
        groups,
        matrix_len(k, n, "grouped B matrix")?,
        "grouped B slab",
    )?;
    let c_len = matrix_len(
        groups,
        matrix_len(m, n, "grouped C matrix")?,
        "grouped C slab",
    )?;
    let a = random_values(a_len, 0x6173);
    let b = random_values(b_len, 0xB17E);
    let c = vec![0.0; c_len];
    let mut problems = Vec::with_capacity(groups);
    for group in 0..groups {
        problems.push(Some(GemmProblem {
            m,
            k,
            n,
            a_offset: group * m * k,
            b_offset: group * k * n,
            c_offset: group * m * n,
        }));
    }
    let mut plan = build_grouped_gemm_plan(ctx, problems, &a, &b, &c)?;
    let flops = 2.0 * groups as f64 * m as f64 * k as f64 * n as f64;
    let blas = new_grouped_blas(ctx)?;
    let result = time_op(op, iters, flops, || {
        execute_grouped_gemm_bench(ctx, &mut plan, blas.as_ref())
    })?;
    validate_output(ctx, &plan)?;
    Ok(result)
}

fn grouped_shape(shape: &[usize]) -> Result<(usize, usize, usize, usize)> {
    match shape {
        [m, k, n] => {
            ensure_nonzero("grouped_gemm", shape)?;
            Ok((1, *m, *k, *n))
        }
        [groups, m, k, n] => {
            ensure_nonzero("grouped_gemm", shape)?;
            Ok((*groups, *m, *k, *n))
        }
        _ => Err(shape_error("grouped_gemm", 4, shape)),
    }
}

fn sync_cuda(ctx: &BenchCudaContext, op: &str) -> Result<()> {
    #[cfg(test)]
    CUDA_SYNC_CALLS.fetch_add(1, Ordering::SeqCst);
    ctx.inner()
        .default_stream()
        .synchronize()
        .map_err(|err| cuda_device_error(ctx, op, format!("microbench sync failed: {err}")))
}

#[cfg(test)]
pub(in crate::autotune) fn reset_cuda_sync_count() {
    CUDA_COSINE_LAUNCH_CALLS.store(0, Ordering::SeqCst);
    CUDA_SYNC_CALLS.store(0, Ordering::SeqCst);
}

#[cfg(test)]
pub(in crate::autotune) fn cuda_cosine_launch_count() -> usize {
    CUDA_COSINE_LAUNCH_CALLS.load(Ordering::SeqCst)
}

#[cfg(test)]
pub(in crate::autotune) fn cuda_sync_count() -> usize {
    CUDA_SYNC_CALLS.load(Ordering::SeqCst)
}

fn cuda_device_error(ctx: &BenchCudaContext, op: &str, detail: String) -> ForgeError {
    ForgeError::DeviceUnavailable {
        device: format!("cuda:{}", ctx.device_idx()),
        detail: format!("{op} {detail}"),
        remediation: CUDA_REQUIRED_REMEDIATION.to_string(),
    }
}
