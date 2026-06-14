use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use proptest::prelude::*;

use super::{CudaAllocError, CudaMalloc, OomGuard};
use crate::vram::{
    BlockDeallocator, BlockId, BlockKind, DevicePtr, GpuBlockRegistry, VramBudgeter, VramProbe,
};
use crate::{ForgeError, Result};

const GIB: usize = 1024 * 1024 * 1024;
const BUDGET_CODE: &str = "CALYX_FORGE_VRAM_BUDGET";
const GPU_CODE: &str = "CALYX_GPU_ERROR";

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
    fn freed(&self) -> Vec<(DevicePtr, usize)> {
        self.freed
            .lock()
            .map(|freed| freed.clone())
            .unwrap_or_default()
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

fn budgeter() -> VramBudgeter<StaticProbe> {
    VramBudgeter::with_soft_cap(GIB, StaticProbe)
}

fn registry_with_blocks<'b>(
    budgeter: &'b VramBudgeter<StaticProbe>,
    dealloc: RecordingDealloc,
    blocks: usize,
) -> Arc<Mutex<GpuBlockRegistry<'b, StaticProbe, RecordingDealloc>>> {
    let mut registry = GpuBlockRegistry::new(budgeter, dealloc, 16);
    for id in 0..blocks {
        let guard = budgeter.reserve(0).expect("zero-byte test reservation");
        registry.insert(
            BlockId(id as u64),
            DevicePtr(0x1000 + id as u64),
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

#[test]
fn malloc_oom_twice_succeeds_after_eviction_retries() -> Result<()> {
    let budgeter = budgeter();
    let dealloc = RecordingDealloc::default();
    let registry = registry_with_blocks(&budgeter, dealloc.clone(), 2);
    let allocator = ScriptedMalloc::new(vec![
        Err(CudaAllocError::MemoryAllocation),
        Err(CudaAllocError::MemoryAllocation),
        Ok(0xfeed_beef),
    ]);
    let guard = OomGuard::new(&budgeter, Arc::clone(&registry), allocator.clone(), 1);

    let ptr = guard.alloc_with_retry(4096)?;

    assert_eq!(ptr as usize, 0xfeed_beef);
    assert_eq!(allocator.calls(), 3);
    assert_eq!(dealloc.freed().len(), 2);
    let stats = budgeter.stats().oom_guard;
    assert_eq!(stats.oom_intercepts, 2);
    assert_eq!(stats.final_failures, 0);
    assert_eq!(
        registry
            .lock()
            .expect("registry lock")
            .stats()
            .resident_blocks,
        0
    );
    Ok(())
}

#[test]
fn malloc_oom_empty_registry_fails_closed_without_looping() {
    let budgeter = budgeter();
    let registry = registry_with_blocks(&budgeter, RecordingDealloc::default(), 0);
    let allocator = ScriptedMalloc::new(vec![
        Err(CudaAllocError::MemoryAllocation),
        Err(CudaAllocError::MemoryAllocation),
        Err(CudaAllocError::MemoryAllocation),
    ]);
    let guard = OomGuard::with_retries(&budgeter, registry, allocator.clone(), 1, 3);

    let err = guard
        .alloc_with_retry(4096)
        .expect_err("empty registry must fail closed");

    assert_eq!(err.code(), BUDGET_CODE);
    assert_eq!(allocator.calls(), 1);
    let stats = budgeter.stats().oom_guard;
    assert_eq!(stats.oom_intercepts, 1);
    assert_eq!(stats.final_failures, 1);
}

#[test]
fn dispatch_reduces_batch_once_then_succeeds() -> Result<()> {
    let budgeter = budgeter();
    let registry = registry_with_blocks(&budgeter, RecordingDealloc::default(), 0);
    let allocator = ScriptedMalloc::new(vec![]);
    let guard = OomGuard::new(&budgeter, registry, allocator, 1);
    let seen = Arc::new(Mutex::new(Vec::new()));
    let seen_for_call = Arc::clone(&seen);

    let output = guard.dispatch_with_retry(64, move |batch| {
        seen_for_call.lock().expect("seen lock").push(batch);
        if batch > 32 {
            Err(budget_error("synthetic dispatch OOM"))
        } else {
            Ok(batch)
        }
    })?;

    assert_eq!(output, 32);
    assert_eq!(*seen.lock().expect("seen lock"), vec![64, 32]);
    assert_eq!(budgeter.stats().oom_guard.batch_reductions, 1);
    Ok(())
}

#[test]
fn dispatch_at_min_batch_fails_without_extra_recursion() {
    let budgeter = budgeter();
    let registry = registry_with_blocks(&budgeter, RecordingDealloc::default(), 0);
    let allocator = ScriptedMalloc::new(vec![]);
    let guard = OomGuard::new(&budgeter, registry, allocator, 1);
    let calls = Arc::new(AtomicUsize::new(0));
    let calls_for_closure = Arc::clone(&calls);

    let err = guard
        .dispatch_with_retry(1, move |_| {
            calls_for_closure.fetch_add(1, Ordering::AcqRel);
            Err::<usize, _>(budget_error("still over budget"))
        })
        .expect_err("min batch must fail closed");

    assert_eq!(err.code(), BUDGET_CODE);
    assert_eq!(calls.load(Ordering::Acquire), 1);
    let stats = budgeter.stats().oom_guard;
    assert_eq!(stats.batch_reductions, 0);
    assert_eq!(stats.final_failures, 1);
}

#[test]
fn max_retries_zero_makes_single_malloc_attempt() {
    let budgeter = budgeter();
    let dealloc = RecordingDealloc::default();
    let registry = registry_with_blocks(&budgeter, dealloc.clone(), 1);
    let allocator =
        ScriptedMalloc::new(vec![Err(CudaAllocError::MemoryAllocation), Ok(0xfeed_beef)]);
    let guard = OomGuard::with_retries(&budgeter, Arc::clone(&registry), allocator.clone(), 1, 0);

    let err = guard
        .alloc_with_retry(4096)
        .expect_err("max_retries=0 must not retry");

    assert_eq!(err.code(), BUDGET_CODE);
    assert_eq!(allocator.calls(), 1);
    assert!(dealloc.freed().is_empty());
    assert_eq!(
        registry
            .lock()
            .expect("registry lock")
            .stats()
            .resident_blocks,
        1
    );
}

#[test]
fn non_oom_cuda_error_maps_to_gpu_error_without_retry() {
    let budgeter = budgeter();
    let dealloc = RecordingDealloc::default();
    let registry = registry_with_blocks(&budgeter, dealloc.clone(), 1);
    let allocator = ScriptedMalloc::new(vec![Err(CudaAllocError::Other {
        code: 700,
        name: "cudaErrorIllegalAddress".into(),
    })]);
    let guard = OomGuard::new(&budgeter, registry, allocator.clone(), 1);

    let err = guard
        .alloc_with_retry(4096)
        .expect_err("non-OOM CUDA error must not retry");

    assert_eq!(err.code(), GPU_CODE);
    assert_eq!(allocator.calls(), 1);
    assert!(dealloc.freed().is_empty());
    assert_eq!(budgeter.stats().oom_guard.oom_intercepts, 0);
}

#[test]
fn all_malloc_attempts_fail_after_exact_eviction_budget() {
    let budgeter = budgeter();
    let dealloc = RecordingDealloc::default();
    let registry = registry_with_blocks(&budgeter, dealloc.clone(), 3);
    let allocator = ScriptedMalloc::new(vec![
        Err(CudaAllocError::MemoryAllocation),
        Err(CudaAllocError::MemoryAllocation),
        Err(CudaAllocError::MemoryAllocation),
        Ok(0xfeed_beef),
    ]);
    let guard = OomGuard::with_retries(&budgeter, registry, allocator.clone(), 1, 3);

    let err = guard
        .alloc_with_retry(4096)
        .expect_err("all allowed attempts fail closed");

    assert_eq!(err.code(), BUDGET_CODE);
    assert_eq!(allocator.calls(), 3);
    assert_eq!(dealloc.freed().len(), 3);
    let stats = budgeter.stats().oom_guard;
    assert_eq!(stats.oom_intercepts, 3);
    assert_eq!(stats.final_failures, 1);
}

proptest! {
    #[test]
    fn dispatch_retry_terminates_within_retry_budget(
        batch in 1usize..=512,
        min_batch in 1usize..=32,
        max_retries in 0u8..=8,
    ) {
        let budgeter = budgeter();
        let registry = registry_with_blocks(&budgeter, RecordingDealloc::default(), 0);
        let allocator = ScriptedMalloc::new(vec![]);
        let guard = OomGuard::with_retries(
            &budgeter,
            registry,
            allocator,
            min_batch,
            max_retries,
        );
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_for_closure = Arc::clone(&calls);

        let _ = guard.dispatch_with_retry(batch, move |_| {
            calls_for_closure.fetch_add(1, Ordering::AcqRel);
            Err::<usize, _>(budget_error("synthetic persistent budget failure"))
        });

        prop_assert!(calls.load(Ordering::Acquire) <= usize::from(max_retries) + 1);
    }
}
