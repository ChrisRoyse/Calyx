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
    let reference = bench_gemm_reference_cublas(&ctx, PERF_DIM, PERF_DIM, PERF_DIM, PERF_ITERS)?;
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
