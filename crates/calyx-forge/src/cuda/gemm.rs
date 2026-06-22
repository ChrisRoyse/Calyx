use std::time::Instant;

use cudarc::cublas::{CudaBlas, Gemm, GemmConfig, sys};
use cudarc::driver::CudaSlice;

use crate::cpu::{check_finite, check_shape_2d};
use crate::{CudaContext, ForgeError, Result};

mod mxfp4_path;
mod mxfp8_path;
pub use mxfp4_path::gemm_mxfp4_fp32_accum;
pub use mxfp8_path::gemm_mxfp8_fp32_accum;

const GEMM_REMEDIATION: &str =
    "Check CUDA/cuBLAS status, dimensions, and device memory; fail closed instead of CPU fallback";
const DEVICE_REMEDIATION: &str = "Check CUDA, CUDA GPU availability, and free VRAM";
const BENCH_WARMUP_ITERS: u32 = 5;

pub fn gemm_cublas(
    ctx: &CudaContext,
    a: &CudaSlice<f32>,
    b: &CudaSlice<f32>,
    m: usize,
    k: usize,
    n: usize,
    out: &mut CudaSlice<f32>,
) -> Result<()> {
    let stream = ctx.inner().default_stream();
    let blas = CudaBlas::new(stream)
        .map_err(|err| device_unavailable(ctx, format!("cuBLAS handle creation failed: {err}")))?;
    gemm_checked_with_blas(ctx, &blas, a, b, GemmDims::new(m, k, n), out)
}

pub fn gemm_host(
    ctx: &CudaContext,
    a: &[f32],
    b: &[f32],
    m: usize,
    k: usize,
    n: usize,
    out: &mut [f32],
) -> Result<()> {
    validate_host_inputs(a, b, m, k, n, out)?;
    out.fill(0.0);
    if m == 0 || n == 0 || k == 0 {
        return Ok(());
    }

    let stream = ctx.inner().default_stream();
    let a_dev = stream
        .clone_htod(a)
        .map_err(|err| device_unavailable(ctx, format!("copy A host-to-device failed: {err}")))?;
    let b_dev = stream
        .clone_htod(b)
        .map_err(|err| device_unavailable(ctx, format!("copy B host-to-device failed: {err}")))?;
    let mut out_dev = stream.alloc_zeros(out.len()).map_err(|err| {
        device_unavailable(
            ctx,
            format!("allocate GEMM output device slice failed: {err}"),
        )
    })?;

    gemm_cublas(ctx, &a_dev, &b_dev, m, k, n, &mut out_dev)?;
    stream
        .synchronize()
        .map_err(|err| device_unavailable(ctx, format!("GEMM stream sync failed: {err}")))?;
    let result = stream.clone_dtoh(&out_dev).map_err(|err| {
        device_unavailable(
            ctx,
            format!("copy GEMM output device-to-host failed: {err}"),
        )
    })?;
    out.copy_from_slice(&result);
    Ok(())
}

pub fn bench_gemm_cublas(
    ctx: &CudaContext,
    m: usize,
    k: usize,
    n: usize,
    iters: u32,
) -> Result<f64> {
    let dims = GemmDims::new(m, k, n);
    let mut bench = BenchBuffers::new(ctx, dims)?;
    let blas = new_blas(ctx)?;
    gemm_checked_with_blas(ctx, &blas, &bench.a, &bench.b, dims, &mut bench.out)?;
    timed_gemm(ctx, &blas, &mut bench, dims, iters, true)
}

pub fn bench_gemm_reference_cublas(
    ctx: &CudaContext,
    m: usize,
    k: usize,
    n: usize,
    iters: u32,
) -> Result<f64> {
    let dims = GemmDims::new(m, k, n);
    let mut bench = BenchBuffers::new(ctx, dims)?;
    let blas = new_blas(ctx)?;
    raw_gemm_with_blas(&blas, &bench.a, &bench.b, dims, &mut bench.out)?;
    timed_gemm(ctx, &blas, &mut bench, dims, iters, false)
}

pub fn probe_allocation(ctx: &CudaContext, requested_bytes: usize) -> Result<()> {
    let (free_bytes, total_bytes) = ctx
        .inner()
        .mem_get_info()
        .map_err(|err| device_unavailable(ctx, format!("VRAM query failed: {err}")))?;
    if requested_bytes > free_bytes {
        return Err(ForgeError::DeviceUnavailable {
            device: device_label(ctx),
            detail: format!(
                "requested_bytes={requested_bytes} exceeds free_bytes={free_bytes} total_bytes={total_bytes}"
            ),
            remediation: DEVICE_REMEDIATION.to_string(),
        });
    }
    Ok(())
}

fn validate_host_inputs(
    a: &[f32],
    b: &[f32],
    m: usize,
    k: usize,
    n: usize,
    out: &[f32],
) -> Result<()> {
    check_shape_2d(a, m, k, "cuda.gemm A")?;
    check_shape_2d(b, k, n, "cuda.gemm B")?;
    check_shape_2d(out, m, n, "cuda.gemm output")?;
    check_finite(a, "cuda.gemm")?;
    check_finite(b, "cuda.gemm")?;
    Ok(())
}

fn gemm_checked_with_blas(
    ctx: &CudaContext,
    blas: &CudaBlas,
    a: &CudaSlice<f32>,
    b: &CudaSlice<f32>,
    dims: GemmDims,
    out: &mut CudaSlice<f32>,
) -> Result<()> {
    check_device_shape(a.len(), dims.m, dims.k, "cuda.gemm device A")?;
    check_device_shape(b.len(), dims.k, dims.n, "cuda.gemm device B")?;
    check_device_shape(out.len(), dims.m, dims.n, "cuda.gemm device output")?;
    if dims.is_zero_work() {
        ctx.inner()
            .default_stream()
            .memset_zeros(out)
            .map_err(|err| device_unavailable(ctx, format!("zero GEMM output failed: {err}")))?;
        return Ok(());
    }
    raw_gemm_with_blas(blas, a, b, dims, out)
}

fn raw_gemm_with_blas(
    blas: &CudaBlas,
    a: &CudaSlice<f32>,
    b: &CudaSlice<f32>,
    dims: GemmDims,
    out: &mut CudaSlice<f32>,
) -> Result<()> {
    let cfg = GemmConfig {
        transa: sys::cublasOperation_t::CUBLAS_OP_N,
        transb: sys::cublasOperation_t::CUBLAS_OP_N,
        m: to_i32(dims.m, "m")?,
        n: to_i32(dims.n, "n")?,
        k: to_i32(dims.k, "k")?,
        alpha: 1.0,
        lda: to_i32(dims.m, "lda")?,
        ldb: to_i32(dims.k, "ldb")?,
        beta: 0.0,
        ldc: to_i32(dims.m, "ldc")?,
    };
    unsafe { blas.gemm(cfg, a, b, out) }
        .map_err(|err| cublas_numerical(format!("cublasSgemm_v2 failed: {err}")))
}

fn timed_gemm(
    ctx: &CudaContext,
    blas: &CudaBlas,
    bench: &mut BenchBuffers,
    dims: GemmDims,
    iters: u32,
    checked: bool,
) -> Result<f64> {
    if iters == 0 {
        return Err(cublas_numerical(
            "iters must be greater than zero".to_string(),
        ));
    }

    ctx.inner()
        .default_stream()
        .synchronize()
        .map_err(|err| device_unavailable(ctx, format!("benchmark warmup sync failed: {err}")))?;
    for _ in 0..BENCH_WARMUP_ITERS {
        if checked {
            gemm_checked_with_blas(ctx, blas, &bench.a, &bench.b, dims, &mut bench.out)?;
        } else {
            raw_gemm_with_blas(blas, &bench.a, &bench.b, dims, &mut bench.out)?;
        }
    }
    ctx.inner().default_stream().synchronize().map_err(|err| {
        device_unavailable(
            ctx,
            format!("benchmark same-path warmup sync failed: {err}"),
        )
    })?;

    let start = Instant::now();
    for _ in 0..iters {
        if checked {
            gemm_checked_with_blas(ctx, blas, &bench.a, &bench.b, dims, &mut bench.out)?;
        } else {
            raw_gemm_with_blas(blas, &bench.a, &bench.b, dims, &mut bench.out)?;
        }
    }
    ctx.inner()
        .default_stream()
        .synchronize()
        .map_err(|err| device_unavailable(ctx, format!("benchmark final sync failed: {err}")))?;

    let elapsed_s = start.elapsed().as_secs_f64();
    if elapsed_s <= 0.0 {
        return Err(cublas_numerical(
            "benchmark elapsed time was zero".to_string(),
        ));
    }
    let flops = 2.0 * dims.m as f64 * dims.k as f64 * dims.n as f64 * iters as f64;
    Ok(flops / elapsed_s / 1_000_000_000.0)
}

#[derive(Clone, Copy)]
struct GemmDims {
    m: usize,
    k: usize,
    n: usize,
}

impl GemmDims {
    fn new(m: usize, k: usize, n: usize) -> Self {
        Self { m, k, n }
    }

    fn is_zero_work(self) -> bool {
        self.m == 0 || self.k == 0 || self.n == 0
    }
}

struct BenchBuffers {
    a: CudaSlice<f32>,
    b: CudaSlice<f32>,
    out: CudaSlice<f32>,
}

impl BenchBuffers {
    fn new(ctx: &CudaContext, dims: GemmDims) -> Result<Self> {
        let a = deterministic_values(dims.m * dims.k, 17, 0.03125);
        let b = deterministic_values(dims.k * dims.n, 23, 0.015625);
        let stream = ctx.inner().default_stream();
        Ok(Self {
            a: stream.clone_htod(&a).map_err(|err| {
                device_unavailable(ctx, format!("benchmark copy A failed: {err}"))
            })?,
            b: stream.clone_htod(&b).map_err(|err| {
                device_unavailable(ctx, format!("benchmark copy B failed: {err}"))
            })?,
            out: stream.alloc_zeros(dims.m * dims.n).map_err(|err| {
                device_unavailable(ctx, format!("benchmark output allocation failed: {err}"))
            })?,
        })
    }
}

fn deterministic_values(len: usize, period: usize, scale: f32) -> Vec<f32> {
    (0..len)
        .map(|idx| (idx % period) as f32 - (period / 2) as f32)
        .map(|value| value * scale)
        .collect()
}

fn new_blas(ctx: &CudaContext) -> Result<CudaBlas> {
    CudaBlas::new(ctx.inner().default_stream())
        .map_err(|err| device_unavailable(ctx, format!("cuBLAS handle creation failed: {err}")))
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
        remediation: format!("cuda.gemm {name} exceeds cuBLAS i32 dimension limit"),
    })
}

fn cublas_numerical(detail: String) -> ForgeError {
    ForgeError::NumericalInvariant {
        op: "gemm_cublas".to_string(),
        detail,
        remediation: GEMM_REMEDIATION.to_string(),
    }
}

fn device_unavailable(ctx: &CudaContext, detail: String) -> ForgeError {
    ForgeError::DeviceUnavailable {
        device: device_label(ctx),
        detail,
        remediation: DEVICE_REMEDIATION.to_string(),
    }
}

fn device_label(ctx: &CudaContext) -> String {
    format!("cuda:{}", ctx.device_idx())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Backend, CpuBackend, CudaBackend};
    use proptest::prelude::*;
    use proptest::test_runner::TestCaseError;

    const PERF_DIM: usize = 512;
    const SMOKE_ITERS: u32 = 10;
    const PERF_ITERS: u32 = 1000;

    fn col_major(row: usize, col: usize, rows: usize) -> usize {
        col * rows + row
    }

    fn identity(size: usize) -> Vec<f32> {
        let mut id = vec![0.0; size * size];
        for idx in 0..size {
            id[col_major(idx, idx, size)] = 1.0;
        }
        id
    }

    fn close_enough(actual: f32, expected: f32) -> bool {
        (actual - expected).abs() <= 1e-3 * expected.abs().max(1.0)
    }

    fn compare_slices(actual: &[f32], expected: &[f32]) {
        for (idx, (a, e)) in actual.iter().zip(expected.iter()).enumerate() {
            assert!(close_enough(*a, *e), "idx={idx} actual={a} expected={e}");
        }
    }

    #[test]
    fn gemm_identity_gpu() -> Result<()> {
        let _guard = crate::cuda::test_lock();
        let ctx = crate::init_cuda(0, false)?;
        let cpu = CpuBackend::new();
        let a = deterministic_values(16, 13, 0.25);
        let id = identity(4);
        let mut cpu_out = vec![0.0; 16];
        let mut gpu_out = vec![0.0; 16];

        cpu.gemm(&a, &id, 4, 4, 4, &mut cpu_out)?;
        CudaBackend::with_context(ctx).gemm(&a, &id, 4, 4, 4, &mut gpu_out)?;

        println!(
            "GEMM_IDENTITY_GPU len={} first={:.6} last={:.6}",
            gpu_out.len(),
            gpu_out[0],
            gpu_out[gpu_out.len() - 1]
        );
        compare_slices(&gpu_out, &cpu_out);
        Ok(())
    }

    #[test]
    fn bench_gemm_cublas_positive_gflops() -> Result<()> {
        let _guard = crate::cuda::test_lock();
        let ctx = crate::init_cuda(0, false)?;
        let gflops = bench_gemm_cublas(&ctx, PERF_DIM, PERF_DIM, PERF_DIM, SMOKE_ITERS)?;
        println!("GEMM_BENCH forge_gflops={gflops:.3}");
        assert!(gflops.is_finite());
        assert!(gflops > 0.0);
        Ok(())
    }

    #[test]
    fn perf_vs_cublas() -> Result<()> {
        let _guard = crate::cuda::test_lock();
        let ctx = crate::init_cuda(0, false)?;
        let _warmup = bench_gemm_reference_cublas(&ctx, PERF_DIM, PERF_DIM, PERF_DIM, SMOKE_ITERS)?;
        let forge = bench_gemm_cublas(&ctx, PERF_DIM, PERF_DIM, PERF_DIM, PERF_ITERS)?;
        let reference =
            bench_gemm_reference_cublas(&ctx, PERF_DIM, PERF_DIM, PERF_DIM, PERF_ITERS)?;
        let ratio = forge / reference;
        println!("GEMM_PERF forge_gflops={forge:.3} cublas_gflops={reference:.3} ratio={ratio:.3}");
        assert!(
            ratio >= 0.90,
            "Forge GEMM ratio={ratio:.3} < 0.90 on sm_120"
        );
        Ok(())
    }

    #[test]
    fn gemm_edges_m_n_k_one_match_cpu() -> Result<()> {
        let _guard = crate::cuda::test_lock();
        for (name, m, k, n) in [("m_one", 1, 3, 2), ("n_one", 3, 2, 1), ("k_one", 3, 1, 2)] {
            let a = deterministic_values(m * k, 11, 0.5);
            let b = deterministic_values(k * n, 7, 0.25);
            let mut cpu_out = vec![0.0; m * n];
            let mut gpu_out = vec![0.0; m * n];
            CpuBackend::new().gemm(&a, &b, m, k, n, &mut cpu_out)?;
            CudaBackend::new()?.gemm(&a, &b, m, k, n, &mut gpu_out)?;
            println!(
                "GEMM_EDGE {name} m={m} k={k} n={n} first={:.6} last={:.6}",
                gpu_out[0],
                gpu_out[gpu_out.len() - 1]
            );
            compare_slices(&gpu_out, &cpu_out);
        }
        Ok(())
    }

    #[test]
    fn oom_probe_fails_closed_for_32_gib_request() -> Result<()> {
        let _guard = crate::cuda::test_lock();
        let ctx = crate::init_cuda(0, false)?;
        let err = probe_allocation(&ctx, 32 * 1024 * 1024 * 1024)
            .expect_err("32 GiB request must fail closed when free VRAM is lower");
        println!("GEMM_OOM_PROBE {err}");
        assert!(matches!(err, ForgeError::DeviceUnavailable { .. }));
        Ok(())
    }

    fn finite_f32() -> impl Strategy<Value = f32> {
        -2.0f32..2.0
    }

    fn matrix_case() -> impl Strategy<Value = (usize, usize, usize, Vec<f32>, Vec<f32>)> {
        (1usize..=32, 1usize..=32, 1usize..=32).prop_flat_map(|(m, k, n)| {
            (
                Just(m),
                Just(k),
                Just(n),
                proptest::collection::vec(finite_f32(), m * k),
                proptest::collection::vec(finite_f32(), k * n),
            )
        })
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(16))]

        #[test]
        fn gemm_matches_cpu_proptest((m, k, n, a, b) in matrix_case()) {
            let _guard = crate::cuda::test_lock();
            let mut cpu_out = vec![0.0; m * n];
            let mut gpu_out = vec![0.0; m * n];
            CpuBackend::new()
                .gemm(&a, &b, m, k, n, &mut cpu_out)
                .map_err(|err| TestCaseError::fail(err.to_string()))?;
            CudaBackend::new()
                .map_err(|err| TestCaseError::fail(err.to_string()))?
                .gemm(&a, &b, m, k, n, &mut gpu_out)
                .map_err(|err| TestCaseError::fail(err.to_string()))?;
            for (actual, expected) in gpu_out.iter().zip(cpu_out.iter()) {
                prop_assert!(close_enough(*actual, *expected));
            }
        }
    }
}
