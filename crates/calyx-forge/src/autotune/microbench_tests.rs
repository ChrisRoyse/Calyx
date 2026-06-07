use std::collections::HashMap;

use proptest::prelude::*;

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
    assert!(result.cv_pct < 20.0, "cv_pct={}", result.cv_pct);
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
    let result = microbench(
        "cosine",
        &config(BackendKind::Cuda),
        &[100_000, 128],
        Some(&ctx),
        5,
    )?;

    assert_positive(result);
    assert!(result.elapsed_ms < 10_000.0);
    println!(
        "microbench_cosine_returns_positive PASSED gflops={:.3} elapsed_ms={:.3} cv_pct={:.3}",
        result.gflops, result.elapsed_ms, result.cv_pct
    );
    Ok(())
}
