use std::time::Instant;

use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;

use crate::quant::{QuantLevel, Quantizer};
use crate::{
    Backend, BackendKind, BestConfig, CpuBackend, ForgeError, Result, TurboQuantCodec, new_seed,
};

#[cfg(feature = "cuda")]
mod cuda_bench;
#[cfg(feature = "cuda")]
use self::cuda_bench::{bench_cuda_cosine, bench_cuda_gemm, bench_grouped_gemm};
#[cfg(feature = "cuda")]
use crate::CudaContext;

#[cfg(feature = "cuda")]
pub type BenchCudaContext = CudaContext;
#[cfg(not(feature = "cuda"))]
pub enum BenchCudaContext {}

const MICROBENCH_REMEDIATION: &str =
    "Use a supported op, non-zero shape, iters > 0, and a CUDA context when benchmarking CUDA";
const CUDA_REQUIRED_REMEDIATION: &str = "Initialize CUDA in a manual verification run and pass Some(&CudaContext), or benchmark BackendKind::Cpu";
const CV_WARN_PCT: f64 = 20.0;
const MIN_SAMPLE_MS: f64 = 2.0;
const MAX_BATCH_RUNS: u32 = 65_536;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BenchResult {
    pub gflops: f64,
    pub elapsed_ms: f64,
    pub cv_pct: f64,
}

pub fn microbench(
    op: &str,
    config: &BestConfig,
    shape: &[usize],
    ctx: Option<&BenchCudaContext>,
    iters: u32,
) -> Result<BenchResult> {
    match op {
        "gemm" => bench_gemm(op, config, shape, ctx, iters),
        "cosine" => bench_cosine(op, config, shape, ctx, iters),
        "grouped_gemm" => bench_grouped_gemm(op, config, shape, ctx, iters),
        "turboquant_encode" => bench_turboquant_encode(op, config, shape, iters),
        _ => Err(unimplemented_op(op)),
    }
}

fn bench_gemm(
    op: &str,
    config: &BestConfig,
    shape: &[usize],
    ctx: Option<&BenchCudaContext>,
    iters: u32,
) -> Result<BenchResult> {
    let (m, k, n) = gemm_shape(shape)?;
    let a_len = matrix_len(m, k, "gemm A")?;
    let b_len = matrix_len(k, n, "gemm B")?;
    let out_len = matrix_len(m, n, "gemm output")?;
    let a = random_values(a_len, 0xA11CE);
    let b = random_values(b_len, 0xB0B);
    let mut out = vec![0.0; out_len];
    let flops = 2.0 * m as f64 * k as f64 * n as f64;

    match config.backend {
        BackendKind::Cpu => {
            let backend = CpuBackend::new();
            time_op(op, iters, flops, || backend.gemm(&a, &b, m, k, n, &mut out))
        }
        BackendKind::Cuda => bench_cuda_gemm(op, ctx, iters, flops, &a, &b, m, k, n, &mut out),
    }
}

fn bench_cosine(
    op: &str,
    config: &BestConfig,
    shape: &[usize],
    ctx: Option<&BenchCudaContext>,
    iters: u32,
) -> Result<BenchResult> {
    let (rows, dim) = cosine_shape(shape)?;
    let candidates_len = matrix_len(rows, dim, "cosine candidates")?;
    let query = random_values(dim, 0xC051);
    let candidates = random_values(candidates_len, 0xCAFE);
    let mut out = vec![0.0; rows];
    let flops = 4.0 * rows as f64 * dim as f64;

    match config.backend {
        BackendKind::Cpu => {
            let backend = CpuBackend::new();
            time_op(op, iters, flops, || {
                backend.cosine(&query, &candidates, dim, &mut out)
            })
        }
        BackendKind::Cuda => {
            bench_cuda_cosine(op, ctx, iters, flops, &query, &candidates, dim, &mut out)
        }
    }
}

fn bench_turboquant_encode(
    op: &str,
    config: &BestConfig,
    shape: &[usize],
    iters: u32,
) -> Result<BenchResult> {
    let (_rows, dim) = turboquant_shape(shape)?;
    let seed = new_seed(dim, b"calyx-autotune-microbench");
    let codec = TurboQuantCodec::new(seed, quant_level(config))?;
    let vector = random_values(dim, 0x7A_B0);
    let flops = 2.0 * dim as f64;

    time_op(op, iters, flops, || {
        let encoded = codec.encode(&vector)?;
        if encoded.bytes.is_empty() {
            return Err(numerical_error(
                op,
                "turboquant encode produced empty bytes",
            ));
        }
        Ok(())
    })
}

#[cfg(not(feature = "cuda"))]
#[allow(clippy::too_many_arguments)]
fn bench_cuda_gemm(
    op: &str,
    _ctx: Option<&BenchCudaContext>,
    _iters: u32,
    _flops: f64,
    _a: &[f32],
    _b: &[f32],
    _m: usize,
    _k: usize,
    _n: usize,
    _out: &mut [f32],
) -> Result<BenchResult> {
    Err(cuda_required(op))
}

#[cfg(not(feature = "cuda"))]
#[allow(clippy::too_many_arguments)]
fn bench_cuda_cosine(
    op: &str,
    _ctx: Option<&BenchCudaContext>,
    _iters: u32,
    _flops: f64,
    _query: &[f32],
    _candidates: &[f32],
    _dim: usize,
    _out: &mut [f32],
) -> Result<BenchResult> {
    Err(cuda_required(op))
}

#[cfg(not(feature = "cuda"))]
fn bench_grouped_gemm(
    op: &str,
    _config: &BestConfig,
    _shape: &[usize],
    _ctx: Option<&BenchCudaContext>,
    _iters: u32,
) -> Result<BenchResult> {
    Err(cuda_required(op))
}

fn time_op<F>(op: &str, iters: u32, flops_per_iter: f64, mut run: F) -> Result<BenchResult>
where
    F: FnMut() -> Result<()>,
{
    if iters == 0 {
        return Err(numerical_error(op, "iters must be greater than zero"));
    }
    if flops_per_iter <= 0.0 || !flops_per_iter.is_finite() {
        return Err(numerical_error(op, "operation count must be positive"));
    }

    run()?;
    let mut batch_runs = 1;
    loop {
        let start = Instant::now();
        for _ in 0..batch_runs {
            run()?;
        }
        let elapsed_ms = start.elapsed().as_secs_f64() * 1_000.0;
        if elapsed_ms > 0.0 && elapsed_ms.is_finite() {
            if elapsed_ms >= MIN_SAMPLE_MS || batch_runs == MAX_BATCH_RUNS {
                break;
            }
            let scale = (MIN_SAMPLE_MS / elapsed_ms).ceil() as u32;
            batch_runs = batch_runs.saturating_mul(scale.max(2)).min(MAX_BATCH_RUNS);
        } else if batch_runs == MAX_BATCH_RUNS {
            return Err(numerical_error(op, "elapsed time was zero or non-finite"));
        } else {
            batch_runs = batch_runs.saturating_mul(2).min(MAX_BATCH_RUNS);
        }
    }
    let mut timings_ms = Vec::with_capacity(iters as usize);
    for _ in 0..iters {
        let start = Instant::now();
        for _ in 0..batch_runs {
            run()?;
        }
        let elapsed_ms = start.elapsed().as_secs_f64() * 1_000.0;
        if elapsed_ms <= 0.0 || !elapsed_ms.is_finite() {
            return Err(numerical_error(op, "elapsed time was zero or non-finite"));
        }
        timings_ms.push(elapsed_ms);
    }
    summarize(op, flops_per_iter * f64::from(batch_runs), &timings_ms)
}

fn summarize(op: &str, flops_per_iter: f64, timings_ms: &[f64]) -> Result<BenchResult> {
    let elapsed_ms: f64 = timings_ms.iter().sum();
    let mean = elapsed_ms / timings_ms.len() as f64;
    let variance = timings_ms
        .iter()
        .map(|elapsed| {
            let delta = elapsed - mean;
            delta * delta
        })
        .sum::<f64>()
        / timings_ms.len() as f64;
    let cv_pct = if timings_ms.len() <= 1 {
        0.0
    } else {
        variance.sqrt() / mean * 100.0
    };
    let mut sorted = timings_ms.to_vec();
    sorted.sort_by(|left, right| left.partial_cmp(right).unwrap());
    let median_ms = sorted[sorted.len() / 2];
    let gflops = flops_per_iter / (median_ms / 1_000.0) / 1_000_000_000.0;
    if cv_pct > CV_WARN_PCT {
        tracing::warn!(
            target: "calyx::forge::autotune",
            op,
            cv_pct,
            threshold_pct = CV_WARN_PCT,
            "microbench result may be noisy"
        );
    }
    Ok(BenchResult {
        gflops,
        elapsed_ms,
        cv_pct,
    })
}

fn gemm_shape(shape: &[usize]) -> Result<(usize, usize, usize)> {
    if shape.len() != 3 {
        return Err(shape_error("gemm", 3, shape));
    }
    ensure_nonzero("gemm", shape)?;
    Ok((shape[0], shape[1], shape[2]))
}

fn cosine_shape(shape: &[usize]) -> Result<(usize, usize)> {
    if shape.len() != 2 {
        return Err(shape_error("cosine", 2, shape));
    }
    ensure_nonzero("cosine", shape)?;
    Ok((shape[0], shape[1]))
}

fn turboquant_shape(shape: &[usize]) -> Result<(usize, usize)> {
    match shape {
        [dim] => {
            ensure_nonzero("turboquant_encode", shape)?;
            Ok((1, *dim))
        }
        [rows, dim] => {
            ensure_nonzero("turboquant_encode", shape)?;
            Ok((*rows, *dim))
        }
        _ => Err(shape_error("turboquant_encode", 1, shape)),
    }
}

fn ensure_nonzero(op: &str, shape: &[usize]) -> Result<()> {
    if let Some(idx) = shape.iter().position(|dim| *dim == 0) {
        return Err(numerical_error(
            op,
            format!("shape dimension at index {idx} must be non-zero"),
        ));
    }
    Ok(())
}

fn matrix_len(rows: usize, cols: usize, name: &str) -> Result<usize> {
    rows.checked_mul(cols)
        .ok_or_else(|| ForgeError::ShapeMismatch {
            expected: vec![rows, cols],
            got: vec![usize::MAX],
            remediation: format!("{name} shape overflows usize; {MICROBENCH_REMEDIATION}"),
        })
}

fn random_values(len: usize, seed: u64) -> Vec<f32> {
    let mut rng = ChaCha8Rng::seed_from_u64(seed);
    (0..len).map(|_| rng.gen_range(-0.5..0.5)).collect()
}

fn quant_level(config: &BestConfig) -> QuantLevel {
    match config.extra.get("level").map(String::as_str) {
        Some("Bits2p5" | "bits2p5" | "2.5") => QuantLevel::Bits2p5,
        _ => QuantLevel::Bits3p5,
    }
}

fn shape_error(op: &str, expected_rank: usize, shape: &[usize]) -> ForgeError {
    ForgeError::ShapeMismatch {
        expected: vec![expected_rank],
        got: vec![shape.len()],
        remediation: format!("{op} microbench shape rank is invalid; {MICROBENCH_REMEDIATION}"),
    }
}

fn numerical_error(op: &str, detail: impl Into<String>) -> ForgeError {
    ForgeError::NumericalInvariant {
        op: format!("microbench::{op}"),
        detail: detail.into(),
        remediation: MICROBENCH_REMEDIATION.to_string(),
    }
}

fn cuda_required(op: &str) -> ForgeError {
    ForgeError::DeviceUnavailable {
        device: "cuda:0".to_string(),
        detail: format!("{op} microbench requires BackendKind::Cuda with a CUDA context"),
        remediation: CUDA_REQUIRED_REMEDIATION.to_string(),
    }
}

fn unimplemented_op(op: &str) -> ForgeError {
    ForgeError::Unimplemented {
        op: op.to_string(),
        remediation: "Implement this Forge microbench op before enabling autotune exploration"
            .into(),
    }
}
