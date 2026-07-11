use std::collections::HashMap;

use proptest::prelude::*;

#[cfg(feature = "cuda")]
use super::microbench::{cuda_cosine_launch_count, cuda_sync_count, reset_cuda_sync_count};
use super::{BenchResult, microbench};
use crate::{BackendKind, BestConfig, ForgeError, Result};

fn config(backend: BackendKind) -> BestConfig {
    BestConfig {
        backend,
        tile_m: 64,
        tile_n: 64,
        tile_k: 32,
        extra: HashMap::new(),
    }
}

fn assert_positive(result: BenchResult) {
    assert!(result.gflops.is_finite());
    assert!(result.elapsed_ms.is_finite());
    assert!(result.cv_pct.is_finite());
    assert!(result.gflops > 0.0);
    assert!(result.elapsed_ms > 0.0);
}

#[test]
fn microbench_cpu_gemm_iters_one_cv_zero() -> Result<()> {
    let result = microbench("gemm", &config(BackendKind::Cpu), &[1, 1, 1], None, 1)?;

    assert_positive(result);
    assert_eq!(result.cv_pct, 0.0);
    println!(
        "microbench_cpu_gemm_iters_one PASSED gflops={:.6} elapsed_ms={:.6} cv_pct={:.3}",
        result.gflops, result.elapsed_ms, result.cv_pct
    );
    Ok(())
}

#[test]
fn microbench_unknown_op_fails_closed() {
    let err = microbench("unknown_op", &config(BackendKind::Cpu), &[1, 1, 1], None, 1)
        .expect_err("unknown op must fail closed");

    assert!(matches!(err, ForgeError::Unimplemented { .. }));
    assert!(err.to_string().starts_with("CALYX_FORGE_UNIMPLEMENTED"));
    println!("microbench_unknown_op PASSED {err}");
}

#[test]
fn microbench_turboquant_encode_returns_positive() -> Result<()> {
    let mut cfg = config(BackendKind::Cpu);
    cfg.extra.insert("level".to_string(), "bits2p5".to_string());
    let result = microbench("turboquant_encode", &cfg, &[64], None, 2)?;

    assert_positive(result);
    println!(
        "microbench_turboquant_encode PASSED gflops={:.6} elapsed_ms={:.6} cv_pct={:.3}",
        result.gflops, result.elapsed_ms, result.cv_pct
    );
    Ok(())
}

#[test]
fn microbench_quant_dot_returns_positive() -> Result<()> {
    let mut cfg = config(BackendKind::Cpu);
    cfg.extra.insert("level".to_string(), "bits3p5".to_string());
    let result = microbench("quant_dot", &cfg, &[8, 64], None, 2)?;

    assert_positive(result);
    println!(
        "microbench_quant_dot PASSED gflops={:.6} elapsed_ms={:.6} cv_pct={:.3}",
        result.gflops, result.elapsed_ms, result.cv_pct
    );
    Ok(())
}

#[test]
fn microbench_quant_dot_zero_rows_fails_closed() {
    let err = microbench("quant_dot", &config(BackendKind::Cpu), &[0, 64], None, 1)
        .expect_err("zero-row quant_dot must fail closed");

    assert!(matches!(err, ForgeError::NumericalInvariant { .. }));
    assert!(err.to_string().contains("microbench::quant_dot"));
    println!("microbench_quant_dot_zero_rows PASSED {err}");
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(4))]

    #[test]
    fn microbench_cpu_gemm_repeated_runs_stay_within_2x(dim in 64usize..=96) {
        let cfg = config(BackendKind::Cpu);
        let shape = [dim, dim, dim];
        let first = microbench("gemm", &cfg, &shape, None, 5)?;
        let second = microbench("gemm", &cfg, &shape, None, 5)?;
        let ratio = (first.gflops / second.gflops).max(second.gflops / first.gflops);

        prop_assert!(first.gflops > 0.0);
        prop_assert!(second.gflops > 0.0);
        println!(
            "microbench_cpu_gemm_repeat dim={dim} first_gflops={:.6} second_gflops={:.6} ratio={ratio:.6} first_cv={:.3} second_cv={:.3}",
            first.gflops, second.gflops, first.cv_pct, second.cv_pct
        );
        prop_assert!(ratio <= 2.0, "ratio={ratio}");
    }
}

#[cfg(feature = "cuda")]
#[test]
fn microbench_gemm_returns_positive_gflops() -> Result<()> {
    let _guard = crate::cuda::test_lock();
    let ctx = crate::init_cuda(0, false)?;
    let result = microbench(
        "gemm",
        &config(BackendKind::Cuda),
        &[1024, 1024, 1024],
        Some(&ctx),
        5,
    )?;

    assert_positive(result);
    assert!(result.elapsed_ms < 10_000.0);
    println!(
        "microbench_gemm_returns_positive_gflops PASSED gflops={:.3} elapsed_ms={:.3} cv_pct={:.3}",
        result.gflops, result.elapsed_ms, result.cv_pct
    );
    Ok(())
}

#[cfg(feature = "cuda")]
#[test]
fn microbench_cosine_returns_positive() -> Result<()> {
    let _guard = crate::cuda::test_lock();
    let ctx = crate::init_cuda(0, false)?;
    reset_cuda_sync_count();
    let result = microbench(
        "cosine",
        &config(BackendKind::Cuda),
        &[100_000, 128],
        Some(&ctx),
        5,
    )?;
    let launch_calls = cuda_cosine_launch_count();
    let sync_calls = cuda_sync_count();

    assert_positive(result);
    assert!(launch_calls > 0, "CUDA cosine microbench must launch work");
    assert_eq!(
        sync_calls, launch_calls,
        "CUDA cosine microbench must synchronize every timed launch"
    );
    assert!(result.elapsed_ms < 10_000.0);
    println!(
        "microbench_cosine_returns_positive PASSED gflops={:.3} elapsed_ms={:.3} cv_pct={:.3} launch_calls={launch_calls} sync_calls={sync_calls}",
        result.gflops, result.elapsed_ms, result.cv_pct,
    );
    Ok(())
}

#[cfg(feature = "cuda")]
#[test]
#[ignore = "requires a CUDA stack with stable cuBLAS grouped GEMM support"]
fn microbench_grouped_gemm_returns_positive() -> Result<()> {
    let _guard = crate::cuda::test_lock();
    let ctx = crate::init_cuda(0, false)?;
    let result = microbench(
        "grouped_gemm",
        &config(BackendKind::Cuda),
        &[2, 128, 128, 128],
        Some(&ctx),
        3,
    )?;

    assert_positive(result);
    assert!(result.elapsed_ms < 10_000.0);
    println!(
        "microbench_grouped_gemm_returns_positive PASSED gflops={:.3} elapsed_ms={:.3} cv_pct={:.3}",
        result.gflops, result.elapsed_ms, result.cv_pct
    );
    Ok(())
}
