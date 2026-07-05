//! #1143 — CUDA BFC arena environment knobs and the shape-diversity gate.
//!
//! `std::env` is process-global, so every test that reads or writes the
//! `CALYX_ONNX_*` arena knobs serializes on one lock.

use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

use super::super::fastembed_runtime::execution_providers;
use super::super::{OnnxProviderPolicy, arena, io_binding};

static ARENA_ENV_LOCK: Mutex<()> = Mutex::new(());

fn arena_env_lock() -> MutexGuard<'static, ()> {
    ARENA_ENV_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn temp_root(name: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("calyx-{name}-{nanos}"));
    std::fs::create_dir_all(&root).unwrap();
    root
}

#[test]
fn execution_provider_policy_is_cuda_fail_loud() {
    let _lock = arena_env_lock();
    let providers = execution_providers(OnnxProviderPolicy::CudaFailLoud).unwrap();

    assert_eq!(providers.len(), 1);
    let provider = format!("{:?}", providers[0]);
    assert!(provider.contains("CUDA"));
    assert!(provider.contains("error_on_failure: true"));
}

#[test]
fn execution_provider_policy_can_be_explicit_cpu() {
    let _lock = arena_env_lock();
    let providers = execution_providers(OnnxProviderPolicy::CpuExplicit).unwrap();

    assert_eq!(providers.len(), 1);
    let provider = format!("{:?}", providers[0]);
    assert!(provider.contains("CPU"));
    assert!(!provider.contains("CUDA"));
}

#[test]
fn cuda_graph_env_enables_cuda_provider_option() {
    let _lock = arena_env_lock();
    unsafe { std::env::set_var("CALYX_ONNX_CUDA_GRAPHS", "1") };
    assert!(io_binding::configured_cuda_graphs().unwrap());
    let providers = execution_providers(OnnxProviderPolicy::CudaFailLoud).unwrap();
    unsafe { std::env::remove_var("CALYX_ONNX_CUDA_GRAPHS") };

    let provider = format!("{:?}", providers[0]);
    assert!(provider.contains("CUDA"));
}

#[test]
fn cuda_graph_env_fails_closed_on_garbage() {
    let _lock = arena_env_lock();
    unsafe { std::env::set_var("CALYX_ONNX_CUDA_GRAPHS", "maybe") };
    let error = execution_providers(OnnxProviderPolicy::CudaFailLoud).unwrap_err();
    assert_eq!(error.code, "CALYX_ONNX_CUDA_GRAPHS_INVALID");
    unsafe { std::env::remove_var("CALYX_ONNX_CUDA_GRAPHS") };
}

#[test]
fn cuda_graphs_require_gpu_policy_and_io_binding() {
    let _lock = arena_env_lock();
    unsafe { std::env::set_var("CALYX_ONNX_CUDA_GRAPHS", "1") };
    let error = io_binding::OnnxRunPlan::new(OnnxProviderPolicy::CpuExplicit, "cpu-graph-test")
        .unwrap_err();
    assert_eq!(error.code, "CALYX_ONNX_CUDA_GRAPHS_CPU_POLICY");

    unsafe { std::env::set_var("CALYX_ONNX_IO_BINDING", "0") };
    let error = io_binding::OnnxRunPlan::new(OnnxProviderPolicy::CudaFailLoud, "graph-no-bind")
        .unwrap_err();
    assert_eq!(error.code, "CALYX_ONNX_CUDA_GRAPHS_IO_BINDING");
    unsafe { std::env::remove_var("CALYX_ONNX_IO_BINDING") };

    unsafe { std::env::set_var("CALYX_ONNX_ARENA_SHRINK", "always") };
    let error =
        io_binding::OnnxRunPlan::new(OnnxProviderPolicy::CudaFailLoud, "graph-shrink").unwrap_err();
    assert_eq!(error.code, "CALYX_ONNX_CUDA_GRAPHS_ARENA_SHRINK");
    unsafe { std::env::remove_var("CALYX_ONNX_ARENA_SHRINK") };
    unsafe { std::env::remove_var("CALYX_ONNX_CUDA_GRAPHS") };
}

#[test]
fn gpu_mem_limit_env_applies_and_fails_closed_on_garbage() {
    let _lock = arena_env_lock();
    // SAFETY: single-threaded within the arena env lock; restored before unlock.
    unsafe { std::env::set_var("CALYX_ONNX_GPU_MEM_LIMIT_MIB", "4096") };
    assert_eq!(
        arena::configured_gpu_mem_limit().unwrap(),
        Some(4096 * 1024 * 1024)
    );
    assert!(execution_providers(OnnxProviderPolicy::CudaFailLoud).is_ok());

    unsafe { std::env::set_var("CALYX_ONNX_GPU_MEM_LIMIT_MIB", "not-a-number") };
    let error = execution_providers(OnnxProviderPolicy::CudaFailLoud).unwrap_err();
    assert_eq!(error.code, "CALYX_ONNX_GPU_MEM_LIMIT_INVALID");

    unsafe { std::env::set_var("CALYX_ONNX_GPU_MEM_LIMIT_MIB", "0") };
    let error = execution_providers(OnnxProviderPolicy::CudaFailLoud).unwrap_err();
    assert_eq!(error.code, "CALYX_ONNX_GPU_MEM_LIMIT_INVALID");

    unsafe { std::env::remove_var("CALYX_ONNX_GPU_MEM_LIMIT_MIB") };
    assert_eq!(arena::configured_gpu_mem_limit().unwrap(), None);
    assert!(execution_providers(OnnxProviderPolicy::CudaFailLoud).is_ok());
}

#[test]
fn gpu_mem_limit_preflight_refuses_artifacts_larger_than_cap() {
    let _lock = arena_env_lock();
    let root = temp_root("onnx-arena-preflight");
    let model = root.join("model.onnx");
    let external = root.join("model.onnx.data");
    std::fs::write(&model, vec![0_u8; 700 * 1024]).unwrap();
    std::fs::write(&external, vec![0_u8; 500 * 1024]).unwrap();
    unsafe { std::env::set_var("CALYX_ONNX_GPU_MEM_LIMIT_MIB", "1") };

    let error = arena::preflight_gpu_mem_limit_for_artifacts(
        "preflight-test",
        OnnxProviderPolicy::CudaFailLoud,
        [model.as_path(), external.as_path()],
    )
    .unwrap_err();
    assert_eq!(error.code, "CALYX_LENS_CONFIG_INVALID");
    assert!(error.message.contains("refused before ONNX session init"));
    assert!(error.message.contains("CALYX_ONNX_GPU_MEM_LIMIT_MIB=1 MiB"));

    unsafe { std::env::remove_var("CALYX_ONNX_GPU_MEM_LIMIT_MIB") };
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn gpu_mem_limit_preflight_skips_cpu_policy() {
    let _lock = arena_env_lock();
    let root = temp_root("onnx-arena-cpu-preflight");
    let model = root.join("model.onnx");
    std::fs::write(&model, vec![0_u8; 2 * 1024 * 1024]).unwrap();
    unsafe { std::env::set_var("CALYX_ONNX_GPU_MEM_LIMIT_MIB", "1") };

    arena::preflight_gpu_mem_limit_for_artifacts(
        "cpu-preflight-test",
        OnnxProviderPolicy::CpuExplicit,
        [model.as_path()],
    )
    .unwrap();

    unsafe { std::env::remove_var("CALYX_ONNX_GPU_MEM_LIMIT_MIB") };
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn arena_shrink_env_fails_closed_on_garbage() {
    let _lock = arena_env_lock();
    unsafe { std::env::set_var("CALYX_ONNX_ARENA_SHRINK", "sometimes") };
    let error = io_binding::OnnxRunPlan::new(OnnxProviderPolicy::CudaFailLoud, "shrink-env-test")
        .unwrap_err();
    assert_eq!(error.code, "CALYX_ONNX_ARENA_SHRINK_INVALID");
    unsafe { std::env::remove_var("CALYX_ONNX_ARENA_SHRINK") };
}

#[test]
fn max_distinct_shapes_env_fails_closed_on_garbage() {
    let _lock = arena_env_lock();
    unsafe { std::env::set_var("CALYX_ONNX_MAX_DISTINCT_SHAPES", "-3") };
    let error =
        io_binding::OnnxRunPlan::new(OnnxProviderPolicy::CudaFailLoud, "shape-limit-env-test")
            .unwrap_err();
    assert_eq!(error.code, "CALYX_ONNX_SHAPE_LIMIT_INVALID");
    unsafe { std::env::remove_var("CALYX_ONNX_MAX_DISTINCT_SHAPES") };
}

#[test]
fn gpu_shape_diversity_fails_loud_past_the_cap() {
    let _lock = arena_env_lock();
    unsafe { std::env::set_var("CALYX_ONNX_MAX_DISTINCT_SHAPES", "4") };
    let mut plan =
        io_binding::OnnxRunPlan::new(OnnxProviderPolicy::CudaFailLoud, "shape-cap-test").unwrap();
    for batch in 1..=4_usize {
        assert!(plan.enforce_shape_contract((batch, 128)).unwrap());
        // Repeats of a seen shape never trip the cap.
        assert!(!plan.enforce_shape_contract((batch, 128)).unwrap());
    }
    let error = plan.enforce_shape_contract((5, 128)).unwrap_err();
    assert_eq!(error.code, "CALYX_ONNX_SHAPE_DIVERSITY");
    unsafe { std::env::remove_var("CALYX_ONNX_MAX_DISTINCT_SHAPES") };
}

#[test]
fn cpu_sessions_do_not_gate_shape_diversity() {
    let _lock = arena_env_lock();
    unsafe { std::env::set_var("CALYX_ONNX_MAX_DISTINCT_SHAPES", "2") };
    let mut plan =
        io_binding::OnnxRunPlan::new(OnnxProviderPolicy::CpuExplicit, "cpu-shape-test").unwrap();
    for batch in 1..=8_usize {
        plan.enforce_shape_contract((batch, 64)).unwrap();
    }
    unsafe { std::env::remove_var("CALYX_ONNX_MAX_DISTINCT_SHAPES") };
}
