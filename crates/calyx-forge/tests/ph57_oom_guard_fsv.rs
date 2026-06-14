use std::collections::VecDeque;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use calyx_forge::{
    BlockDeallocator, BlockId, BlockKind, CudaAllocError, CudaMalloc, DevicePtr, ForgeError,
    GpuBlockRegistry, OomGuard, Result, VramBudgeter, VramProbe,
};
use serde::Serialize;

const GIB: usize = 1024 * 1024 * 1024;
#[cfg(feature = "cuda")]
const MIB: usize = 1024 * 1024;

struct StaticProbe;
impl VramProbe for StaticProbe {
    fn free_device_vram(&self) -> Result<usize> {
        Ok(64 * GIB)
    }
}

#[derive(Clone, Default)]
struct RecordingDealloc {
    freed: Arc<Mutex<Vec<(DevicePtr, usize)>>>,
}

impl RecordingDealloc {
    fn freed_count(&self) -> usize {
        self.freed.lock().map(|freed| freed.len()).unwrap_or(0)
    }
}

impl BlockDeallocator for RecordingDealloc {
    fn free(&self, ptr: DevicePtr, size_bytes: usize) -> Result<()> {
        if let Ok(mut freed) = self.freed.lock() {
            freed.push((ptr, size_bytes));
        }
        Ok(())
    }
}

#[derive(Clone)]
struct ScriptedMalloc {
    script: Arc<Mutex<VecDeque<std::result::Result<usize, CudaAllocError>>>>,
    calls: Arc<AtomicUsize>,
}

impl ScriptedMalloc {
    fn new(script: Vec<std::result::Result<usize, CudaAllocError>>) -> Self {
        Self {
            script: Arc::new(Mutex::new(script.into())),
            calls: Arc::new(AtomicUsize::new(0)),
        }
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::Acquire)
    }
}

impl CudaMalloc for ScriptedMalloc {
    fn cuda_malloc(&self, _size: usize) -> std::result::Result<*mut u8, CudaAllocError> {
        self.calls.fetch_add(1, Ordering::AcqRel);
        let next = self
            .script
            .lock()
            .map_err(|_| CudaAllocError::Other {
                code: -1,
                name: "poisoned-script".into(),
            })?
            .pop_front()
            .unwrap_or(Err(CudaAllocError::MemoryAllocation));
        next.map(|addr| addr as *mut u8)
    }
}

#[derive(Serialize)]
struct Readback {
    before: calyx_forge::VramStats,
    after_alloc_retry: calyx_forge::VramStats,
    after_dispatch_success: calyx_forge::VramStats,
    after_final_failure: calyx_forge::VramStats,
    happy_path_ptr: usize,
    alloc_calls: usize,
    evictions: usize,
    dispatch_output: usize,
    final_error_code: String,
    non_oom_error_code: String,
}

#[cfg(feature = "cuda")]
#[derive(Serialize)]
struct CudaReadback {
    before: calyx_forge::VramStats,
    after: calyx_forge::VramStats,
    requested_bytes: usize,
    device_total_bytes: usize,
    error_code: String,
}

#[test]
fn ph57_oom_guard_writes_readback_artifacts() -> Result<()> {
    let budgeter = VramBudgeter::with_soft_cap(GIB, StaticProbe);
    let dealloc = RecordingDealloc::default();
    let registry = registry_with_blocks(&budgeter, dealloc.clone(), 5);
    let before = budgeter.stats();

    let allocator = ScriptedMalloc::new(vec![
        Err(CudaAllocError::MemoryAllocation),
        Err(CudaAllocError::MemoryAllocation),
        Ok(0xabcddcba),
    ]);
    let guard = OomGuard::new(&budgeter, Arc::clone(&registry), allocator.clone(), 1);
    let ptr = guard.alloc_with_retry(4096)?;
    let after_alloc_retry = budgeter.stats();

    let dispatch_output = guard.dispatch_with_retry(64, |batch| {
        if batch > 32 {
            Err(budget_error("synthetic dispatch OOM"))
        } else {
            Ok(batch)
        }
    })?;
    let after_dispatch_success = budgeter.stats();

    let failing_allocator = ScriptedMalloc::new(vec![
        Err(CudaAllocError::MemoryAllocation),
        Err(CudaAllocError::MemoryAllocation),
        Err(CudaAllocError::MemoryAllocation),
    ]);
    let failing_guard =
        OomGuard::with_retries(&budgeter, Arc::clone(&registry), failing_allocator, 1, 3);
    let final_error = failing_guard
        .alloc_with_retry(4096)
        .expect_err("persistent CUDA OOM must fail closed");
    let after_final_failure = budgeter.stats();

    let non_oom_allocator = ScriptedMalloc::new(vec![Err(CudaAllocError::Other {
        code: 700,
        name: "cudaErrorIllegalAddress".into(),
    })]);
    let non_oom_guard = OomGuard::new(&budgeter, Arc::clone(&registry), non_oom_allocator, 1);
    let non_oom_error = non_oom_guard
        .alloc_with_retry(4096)
        .expect_err("non-OOM CUDA error must remain a GPU error");

    assert_eq!(before.oom_guard.oom_intercepts, 0);
    assert!(after_final_failure.oom_guard.oom_intercepts > 0);
    assert_eq!(dispatch_output, 32);
    assert_eq!(final_error.code(), "CALYX_FORGE_VRAM_BUDGET");
    assert_eq!(non_oom_error.code(), "CALYX_GPU_ERROR");

    let readback = Readback {
        before,
        after_alloc_retry,
        after_dispatch_success,
        after_final_failure,
        happy_path_ptr: ptr as usize,
        alloc_calls: allocator.calls(),
        evictions: dealloc.freed_count(),
        dispatch_output,
        final_error_code: final_error.code().into(),
        non_oom_error_code: non_oom_error.code().into(),
    };

    let root = fsv_root();
    fs::create_dir_all(&root).map_err(io_error)?;
    let json_path = root.join("ph57-oom-guard-readback.json");
    let prom_path = root.join("ph57-oom-guard.prom");
    let json = serde_json::to_string_pretty(&readback).map_err(|err| ForgeError::CacheError {
        op: "serialize oom guard readback".into(),
        path: json_path.display().to_string(),
        detail: err.to_string(),
        remediation: "fix FSV serialization".into(),
    })?;
    fs::write(&json_path, json).map_err(io_error)?;
    fs::write(&prom_path, budgeter.stats().admission_metrics_text()).map_err(io_error)?;

    println!("PH57_OOM_GUARD_JSON {}", json_path.display());
    println!("PH57_OOM_GUARD_PROM {}", prom_path.display());
    Ok(())
}

#[cfg(feature = "cuda")]
#[test]
fn ph57_real_cuda_oom_guard_intercepts_driver_oom() -> Result<()> {
    let ctx = Arc::new(calyx_forge::init_cuda(0, true)?);
    let probe = calyx_forge::CudaVramProbe::new(Arc::clone(&ctx));
    let budgeter = VramBudgeter::with_soft_cap(usize::MAX / 2, probe);
    let registry = registry_with_blocks(&budgeter, RecordingDealloc::default(), 0);
    let guard = OomGuard::with_retries(
        &budgeter,
        registry,
        calyx_forge::RawCudaMalloc::new(Arc::clone(&ctx)),
        1,
        1,
    );

    let before = budgeter.stats();
    let device_total_bytes = (ctx.total_mem_mib() as usize).saturating_mul(MIB);
    let requested_bytes = device_total_bytes.saturating_add(GIB);
    let err = guard
        .alloc_with_retry(requested_bytes)
        .expect_err("oversized real CUDA allocation must fail closed");
    let after = budgeter.stats();

    assert_eq!(err.code(), "CALYX_FORGE_VRAM_BUDGET");
    assert_eq!(before.oom_guard.oom_intercepts, 0);
    assert!(after.oom_guard.oom_intercepts > 0);
    assert_eq!(after.oom_guard.final_failures, 1);

    let root = fsv_root();
    fs::create_dir_all(&root).map_err(io_error)?;
    let path = root.join("ph57-oom-guard-cuda-readback.json");
    let readback = CudaReadback {
        before,
        after,
        requested_bytes,
        device_total_bytes,
        error_code: err.code().into(),
    };
    let json = serde_json::to_string_pretty(&readback).map_err(|err| ForgeError::CacheError {
        op: "serialize CUDA oom guard readback".into(),
        path: path.display().to_string(),
        detail: err.to_string(),
        remediation: "fix FSV serialization".into(),
    })?;
    fs::write(&path, json).map_err(io_error)?;
    println!("PH57_OOM_GUARD_CUDA_JSON {}", path.display());
    Ok(())
}

fn registry_with_blocks<'b, P: VramProbe>(
    budgeter: &'b VramBudgeter<P>,
    dealloc: RecordingDealloc,
    blocks: usize,
) -> Arc<Mutex<GpuBlockRegistry<'b, P, RecordingDealloc>>> {
    let mut registry = GpuBlockRegistry::new(budgeter, dealloc, 16);
    for id in 0..blocks {
        let guard = budgeter.reserve(0).expect("zero-byte test reservation");
        registry.insert(
            BlockId(id as u64),
            DevicePtr(0x2000 + id as u64),
            0,
            BlockKind::General,
            guard,
        );
    }
    Arc::new(Mutex::new(registry))
}

fn budget_error(detail: &str) -> ForgeError {
    ForgeError::VramBudget {
        detail: detail.into(),
        remediation: "test".into(),
    }
}

fn fsv_root() -> PathBuf {
    std::env::var_os("CALYX_FSV_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("calyx-ph57-oom-guard-fsv"))
}

fn io_error(err: std::io::Error) -> ForgeError {
    ForgeError::CacheError {
        op: "write oom guard FSV artifact".into(),
        path: fsv_root().display().to_string(),
        detail: err.to_string(),
        remediation: "ensure CALYX_FSV_ROOT is writable".into(),
    }
}
