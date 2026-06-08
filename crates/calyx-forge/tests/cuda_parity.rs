use std::sync::Mutex;

#[cfg(feature = "cuda")]
use calyx_forge::{
    Backend, CpuBackend, CudaBackend,
    cuda::{bench_gemm_cublas, bench_gemm_reference_cublas},
    init_cuda,
};
mod cuda_parity_support;
#[cfg(feature = "cuda")]
use cuda_parity_support::{
    PARITY_ABS_TOL, l2_norm, load_golden_f32, load_manifest, parity_report, write_cuda_fsv_readback,
};
use cuda_parity_support::{PARITY_TOL, assert_parity, max_rel_err};
use proptest::prelude::*;

#[cfg(feature = "cuda")]
const PERF_DIM: usize = 512;
#[cfg(feature = "cuda")]
const PERF_ITERS: u32 = 5;
#[cfg(feature = "cuda")]
static CUDA_PARITY_LOCK: Mutex<()> = Mutex::new(());
static PANIC_HOOK_LOCK: Mutex<()> = Mutex::new(());

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
    assert_parity(&[1e-8], &[0.0], "edge_near_zero_abs_floor", PARITY_TOL);

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

        let report = parity_report(&cpu, &gpu);
        assert_parity(&cpu, &gpu, "gemm", PARITY_TOL);
        write_cuda_fsv_readback(
            "cuda-gemm-parity.json",
            &serde_json::json!({
                "op": "gemm",
                "relative_tolerance": PARITY_TOL,
                "absolute_tolerance": PARITY_ABS_TOL,
                "max_rel_err": report.max_rel_err,
                "worst_rel_idx": report.worst_rel_idx,
                "worst_rel_cpu": cpu[report.worst_rel_idx],
                "worst_rel_gpu": gpu[report.worst_rel_idx],
                "max_abs_err": report.max_abs_err,
                "worst_abs_idx": report.worst_abs_idx,
                "worst_abs_cpu": cpu[report.worst_abs_idx],
                "worst_abs_gpu": gpu[report.worst_abs_idx],
                "passed_by": if report.max_rel_err <= PARITY_TOL { "relative" } else { "absolute_near_zero" },
            }),
        );
        println!(
            "golden_gemm_parity PASSED rel_err={:.8e} abs_err={:.8e}",
            report.max_rel_err, report.max_abs_err
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

        let report = parity_report(&cpu, &gpu);
        assert_parity(&cpu, &gpu, "normalize", PARITY_TOL);
        write_cuda_fsv_readback(
            "cuda-normalize-parity.json",
            &serde_json::json!({
                "op": "normalize",
                "backend_path": "CudaBackend::normalize",
                "gpu_kernel": "normalize_rows_f32",
                "dim": manifest.dim,
                "manifest_n_vecs": manifest.n_vecs,
                "sample_count": cpu.len() / manifest.dim,
                "rel_err": report.max_rel_err,
                "worst_idx": report.worst_rel_idx,
                "abs_err": report.max_abs_err,
                "worst_abs_idx": report.worst_abs_idx,
                "cpu_first_norm": l2_norm(&cpu[..manifest.dim]),
                "gpu_first_norm": l2_norm(&gpu[..manifest.dim]),
                "cpu_first_4": &cpu[..4],
                "gpu_first_4": &gpu[..4],
            }),
        );
        println!(
            "golden_normalize_parity PASSED rel_err={:.8e} abs_err={:.8e}",
            report.max_rel_err, report.max_abs_err
        );
    }
}

#[test]
#[cfg_attr(not(feature = "cuda"), ignore)]
fn cuda_normalize_fail_closed_edges() {
    #[cfg(feature = "cuda")]
    {
        let _guard = CUDA_PARITY_LOCK
            .lock()
            .unwrap_or_else(|err| err.into_inner());
        let backend = CudaBackend::new().expect("cuda backend");

        let mut zero = vec![0.0, 0.0];
        let zero_err = backend
            .normalize(&mut zero, 2)
            .expect_err("zero vector must fail closed");
        assert!(
            zero_err
                .to_string()
                .starts_with("CALYX_FORGE_NUMERICAL_INVARIANT")
        );
        assert_eq!(zero, vec![0.0, 0.0]);

        let mut non_finite = vec![1.0, f32::INFINITY];
        let non_finite_err = backend
            .normalize(&mut non_finite, 2)
            .expect_err("non-finite vector must fail closed");
        assert!(
            non_finite_err
                .to_string()
                .starts_with("CALYX_FORGE_NUMERICAL_INVARIANT")
        );
        assert_eq!(non_finite, vec![1.0, f32::INFINITY]);

        let mut bad_shape = vec![1.0, 2.0, 3.0];
        let shape_err = backend
            .normalize(&mut bad_shape, 2)
            .expect_err("ragged rows must fail closed");
        assert!(
            shape_err
                .to_string()
                .starts_with("CALYX_FORGE_SHAPE_MISMATCH")
        );

        println!(
            "cuda_normalize_fail_closed_edges PASSED zero={} non_finite={} shape={}",
            zero_err.code(),
            non_finite_err.code(),
            shape_err.code()
        );
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
