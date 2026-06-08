use std::sync::Mutex;
#[cfg(feature = "cuda")]
use std::{fs, path::PathBuf};

#[cfg(feature = "cuda")]
use calyx_forge::{
    Backend, CpuBackend, CudaBackend,
    cuda::{bench_gemm_cublas, bench_gemm_reference_cublas},
    init_cuda,
};
use proptest::prelude::*;
#[cfg(feature = "cuda")]
use serde::Deserialize;

#[cfg(feature = "cuda")]
const GOLDEN_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/golden");
const PARITY_TOL: f32 = 1e-3;
#[cfg(feature = "cuda")]
const PERF_DIM: usize = 512;
#[cfg(feature = "cuda")]
const PERF_ITERS: u32 = 5;
#[cfg(feature = "cuda")]
static CUDA_PARITY_LOCK: Mutex<()> = Mutex::new(());
static PANIC_HOOK_LOCK: Mutex<()> = Mutex::new(());

#[cfg(feature = "cuda")]
#[derive(Debug, Deserialize)]
struct GoldenManifest {
    n_vecs: usize,
    dim: usize,
    gemm_m: usize,
    gemm_k: usize,
    gemm_n: usize,
    topk: usize,
}

#[cfg(feature = "cuda")]
fn golden_path(name: &str) -> PathBuf {
    PathBuf::from(GOLDEN_DIR).join(format!("{name}.bin"))
}

#[cfg(feature = "cuda")]
fn load_golden_f32(name: &str) -> Vec<f32> {
    let path = golden_path(name);
    let bytes = fs::read(&path).unwrap_or_else(|err| panic!("{}: {err}", path.display()));
    if !bytes.len().is_multiple_of(4) {
        panic!("{name}: unexpected EOF in f32 little-endian bytes");
    }
    bytes
        .chunks_exact(4)
        .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect()
}

#[cfg(feature = "cuda")]
fn load_manifest() -> GoldenManifest {
    let path = PathBuf::from(GOLDEN_DIR).join("golden_manifest.json");
    let text = fs::read_to_string(&path).unwrap_or_else(|err| panic!("{}: {err}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|err| panic!("{}: {err}", path.display()))
}

#[cfg(feature = "cuda")]
fn l2_norm(values: &[f32]) -> f32 {
    values.iter().map(|value| value * value).sum::<f32>().sqrt()
}

#[cfg(feature = "cuda")]
fn write_cuda_fsv_readback(file_name: &str, value: &serde_json::Value) {
    let Ok(root) = std::env::var("CALYX_FSV_ROOT") else {
        return;
    };
    let path = PathBuf::from(root).join(file_name);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap_or_else(|err| panic!("{}: {err}", parent.display()));
    }
    let bytes = serde_json::to_vec_pretty(value).expect("serialize cuda fsv readback");
    fs::write(&path, bytes).unwrap_or_else(|err| panic!("{}: {err}", path.display()));
    println!("CUDA_NORMALIZE_READBACK={}", path.display());
}

fn max_rel_err(a: &[f32], b: &[f32]) -> f32 {
    worst_rel_err(a, b).1
}

fn assert_parity(cpu: &[f32], gpu: &[f32], op: &str, tol: f32) {
    assert_eq!(
        cpu.len(),
        gpu.len(),
        "PARITY FAIL op={op} len cpu={} gpu={}",
        cpu.len(),
        gpu.len()
    );
    let (worst_idx, err) = worst_rel_err(cpu, gpu);
    println!("PARITY op={op} rel_err={err:.8e} worst_idx={worst_idx}");
    if err > tol {
        panic!(
            "PARITY FAIL op={op} max_rel_err={err:.2e} > tol={tol:.2e} at index {worst_idx} cpu={} gpu={}",
            cpu[worst_idx], gpu[worst_idx]
        );
    }
}

fn worst_rel_err(a: &[f32], b: &[f32]) -> (usize, f32) {
    a.iter()
        .zip(b.iter())
        .enumerate()
        .map(|(index, (left, right))| {
            let err = (left - right).abs() / (right.abs() + 1e-8);
            (index, err)
        })
        .max_by(|left, right| left.1.total_cmp(&right.1))
        .unwrap_or((0, 0.0))
}

#[test]
#[cfg_attr(not(feature = "cuda"), ignore)]
fn max_rel_err_identical_is_zero() {
    assert_eq!(max_rel_err(&[1.0, 2.0], &[1.0, 2.0]), 0.0);
    println!("max_rel_err_identical PASSED rel_err=0.00000000e0");
}

#[test]
#[cfg_attr(not(feature = "cuda"), ignore)]
fn max_rel_err_known_delta() {
    let err = max_rel_err(&[1.0], &[1.001]);
    println!("max_rel_err_known_delta PASSED rel_err={err:.8e}");
    assert!((err - 0.001).abs() <= 1e-6, "{err}");
}

#[test]
#[cfg_attr(not(feature = "cuda"), ignore)]
fn assert_parity_panics_on_large_error() {
    let _guard = PANIC_HOOK_LOCK
        .lock()
        .unwrap_or_else(|err| err.into_inner());
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let panic =
        std::panic::catch_unwind(|| assert_parity(&[1.002], &[1.0], "synthetic_fail", 1e-3));
    std::panic::set_hook(hook);

    let message = panic_message(panic.expect_err("large parity error must panic"));
    assert!(message.contains("PARITY FAIL"), "{message}");
    println!("assert_parity_fail_closed PASSED");
}

#[test]
#[cfg_attr(not(feature = "cuda"), ignore)]
fn parity_edges_one_near_zero_and_topk_tie() {
    assert_parity(&[2.0005], &[2.0], "edge_one", PARITY_TOL);
    let near_zero = max_rel_err(&[1e-9], &[0.0]);
    println!("PARITY edge_near_zero rel_err={near_zero:.8e}");
    assert!((near_zero - 0.1).abs() <= 1e-6, "{near_zero}");

    #[cfg(feature = "cuda")]
    {
        let _guard = CUDA_PARITY_LOCK
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let scores = [1.0, 2.0, 2.0, 0.5];
        let cpu = CpuBackend::new().topk(&scores, 2).expect("cpu tie topk");
        let gpu = CudaBackend::new()
            .expect("cuda backend")
            .topk(&scores, 2)
            .expect("gpu tie topk");
        println!("golden_topk_tie_parity PASSED cpu={cpu:?} gpu={gpu:?}");
        assert_eq!(cpu, gpu);
    }
}

#[test]
#[cfg_attr(not(feature = "cuda"), ignore)]
fn golden_gemm_parity() {
    #[cfg(feature = "cuda")]
    {
        let _guard = CUDA_PARITY_LOCK
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let manifest = load_manifest();
        let a = load_golden_f32("gemm_A");
        let b = load_golden_f32("gemm_B");
        let mut cpu = vec![0.0; manifest.gemm_m * manifest.gemm_n];
        let mut gpu = vec![0.0; manifest.gemm_m * manifest.gemm_n];

        CpuBackend::new()
            .gemm(
                &a,
                &b,
                manifest.gemm_m,
                manifest.gemm_k,
                manifest.gemm_n,
                &mut cpu,
            )
            .expect("cpu golden gemm");
        CudaBackend::new()
            .expect("cuda backend")
            .gemm(
                &a,
                &b,
                manifest.gemm_m,
                manifest.gemm_k,
                manifest.gemm_n,
                &mut gpu,
            )
            .expect("gpu golden gemm");

        assert_parity(&cpu, &gpu, "gemm", PARITY_TOL);
        println!(
            "golden_gemm_parity PASSED rel_err={:.8e}",
            max_rel_err(&cpu, &gpu)
        );
    }
}

#[test]
#[cfg_attr(not(feature = "cuda"), ignore)]
fn golden_cosine_parity() {
    #[cfg(feature = "cuda")]
    {
        let _guard = CUDA_PARITY_LOCK
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let manifest = load_manifest();
        let vectors = load_golden_f32("vectors_128d");
        let query = &vectors[..manifest.dim];
        let candidates = &vectors[manifest.dim..];
        let mut cpu = vec![0.0; manifest.n_vecs - 1];
        let mut gpu = vec![0.0; manifest.n_vecs - 1];

        CpuBackend::new()
            .cosine(query, candidates, manifest.dim, &mut cpu)
            .expect("cpu golden cosine");
        CudaBackend::new()
            .expect("cuda backend")
            .cosine(query, candidates, manifest.dim, &mut gpu)
            .expect("gpu golden cosine");

        assert_parity(&cpu, &gpu, "cosine", PARITY_TOL);
        println!(
            "golden_cosine_parity PASSED rel_err={:.8e}",
            max_rel_err(&cpu, &gpu)
        );
    }
}

#[test]
#[cfg_attr(not(feature = "cuda"), ignore)]
fn golden_dot_parity() {
    #[cfg(feature = "cuda")]
    {
        let _guard = CUDA_PARITY_LOCK
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let manifest = load_manifest();
        let vectors = load_golden_f32("vectors_128d");
        let query = &vectors[..manifest.dim];
        let candidates = &vectors[manifest.dim..];
        let mut cpu = vec![0.0; manifest.n_vecs - 1];
        let mut gpu = vec![0.0; manifest.n_vecs - 1];

        CpuBackend::new()
            .dot(query, candidates, manifest.dim, &mut cpu)
            .expect("cpu golden dot");
        CudaBackend::new()
            .expect("cuda backend")
            .dot(query, candidates, manifest.dim, &mut gpu)
            .expect("gpu golden dot");

        assert_parity(&cpu, &gpu, "dot", PARITY_TOL);
        println!(
            "golden_dot_parity PASSED rel_err={:.8e}",
            max_rel_err(&cpu, &gpu)
        );
    }
}

#[test]
#[cfg_attr(not(feature = "cuda"), ignore)]
fn golden_l2_parity() {
    #[cfg(feature = "cuda")]
    {
        let _guard = CUDA_PARITY_LOCK
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let manifest = load_manifest();
        let vectors = load_golden_f32("vectors_128d");
        let query = &vectors[..manifest.dim];
        let candidates = &vectors[manifest.dim..];
        let mut cpu = vec![0.0; manifest.n_vecs - 1];
        let mut gpu = vec![0.0; manifest.n_vecs - 1];

        CpuBackend::new()
            .l2(query, candidates, manifest.dim, &mut cpu)
            .expect("cpu golden l2");
        CudaBackend::new()
            .expect("cuda backend")
            .l2(query, candidates, manifest.dim, &mut gpu)
            .expect("gpu golden l2");

        assert_parity(&cpu, &gpu, "l2", PARITY_TOL);
        println!(
            "golden_l2_parity PASSED rel_err={:.8e}",
            max_rel_err(&cpu, &gpu)
        );
    }
}

#[test]
#[cfg_attr(not(feature = "cuda"), ignore)]
fn golden_normalize_parity() {
    #[cfg(feature = "cuda")]
    {
        let _guard = CUDA_PARITY_LOCK
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let manifest = load_manifest();
        let vectors = load_golden_f32("vectors_128d");
        let mut cpu = vectors.clone();
        let mut gpu = vectors;

        CpuBackend::new()
            .normalize(&mut cpu, manifest.dim)
            .expect("cpu golden normalize");
        CudaBackend::new()
            .expect("cuda backend")
            .normalize(&mut gpu, manifest.dim)
            .expect("gpu golden normalize");

        let (worst_idx, rel_err) = worst_rel_err(&cpu, &gpu);
        assert_parity(&cpu, &gpu, "normalize", PARITY_TOL);
        write_cuda_fsv_readback(
            "cuda-normalize-parity.json",
            &serde_json::json!({
                "op": "normalize",
                "dim": manifest.dim,
                "manifest_n_vecs": manifest.n_vecs,
                "sample_count": cpu.len() / manifest.dim,
                "rel_err": rel_err,
                "worst_idx": worst_idx,
                "cpu_first_norm": l2_norm(&cpu[..manifest.dim]),
                "gpu_first_norm": l2_norm(&gpu[..manifest.dim]),
                "cpu_first_4": &cpu[..4],
                "gpu_first_4": &gpu[..4],
            }),
        );
        println!("golden_normalize_parity PASSED rel_err={:.8e}", rel_err);
    }
}

#[test]
#[cfg_attr(not(feature = "cuda"), ignore)]
fn golden_topk_parity() {
    #[cfg(feature = "cuda")]
    {
        let _guard = CUDA_PARITY_LOCK
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let manifest = load_manifest();
        let scores = load_golden_f32("cosine_ref");
        let expected = load_golden_f32("topk_ref");
        let cpu = CpuBackend::new()
            .topk(&scores, manifest.topk)
            .expect("cpu golden topk");
        let gpu = CudaBackend::new()
            .expect("cuda backend")
            .topk(&scores, manifest.topk)
            .expect("gpu golden topk");
        let cpu_indices: Vec<usize> = cpu.iter().map(|(index, _)| *index).collect();
        let gpu_indices: Vec<usize> = gpu.iter().map(|(index, _)| *index).collect();
        let expected_indices: Vec<usize> = expected.iter().map(|index| *index as usize).collect();

        println!(
            "golden_topk_parity PASSED cpu={cpu_indices:?} gpu={gpu_indices:?} expected={expected_indices:?}"
        );
        assert_eq!(
            cpu_indices, gpu_indices,
            "PARITY FAIL op=topk cpu_indices={cpu_indices:?} gpu_indices={gpu_indices:?}"
        );
        assert_eq!(cpu_indices, expected_indices);
    }
}

#[test]
#[cfg_attr(not(feature = "cuda"), ignore)]
fn perf_vs_cublas() {
    #[cfg(feature = "cuda")]
    {
        let _guard = CUDA_PARITY_LOCK
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let ctx = init_cuda(0, false).expect("cuda context");
        let forge =
            bench_gemm_cublas(&ctx, PERF_DIM, PERF_DIM, PERF_DIM, PERF_ITERS).expect("forge bench");
        let reference = bench_gemm_reference_cublas(&ctx, PERF_DIM, PERF_DIM, PERF_DIM, PERF_ITERS)
            .expect("reference bench");
        let ratio = forge / reference;
        println!(
            "perf_vs_cublas PASSED forge_gflops={forge:.3} cublas_gflops={reference:.3} forge_ratio={ratio:.3}"
        );
        assert!(
            ratio >= 0.90,
            "forge_ratio={ratio:.3} < 0.90 (10% cuBLAS gate) on sm_120"
        );
    }
}

proptest! {
    #[test]
    #[cfg_attr(not(feature = "cuda"), ignore)]
    fn max_rel_err_self_is_zero_for_finite_nonzero(
        value in (-1.0e6f32..1.0e6).prop_filter("finite non-zero", |value| {
            value.is_finite() && value.abs() > 1.0e-12
        })
    ) {
        prop_assert_eq!(max_rel_err(&[value], &[value]), 0.0);
    }
}

fn panic_message(panic: Box<dyn std::any::Any + Send>) -> String {
    if let Some(message) = panic.downcast_ref::<String>() {
        return message.clone();
    }
    if let Some(message) = panic.downcast_ref::<&'static str>() {
        return (*message).to_string();
    }
    "<non-string panic>".to_string()
}
