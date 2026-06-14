//! Last-resort OOM guard for Forge CUDA allocation and dispatch.
//!
//! Admission control prevents expected over-budget work from reaching CUDA.
//! This guard catches the remaining race: the device can run out of VRAM
//! between the live `cudaMemGetInfo` read and the actual allocation. CUDA OOM
//! becomes `CALYX_FORGE_VRAM_BUDGET`; other CUDA failures remain GPU errors.

use std::sync::{Arc, Mutex};

use crate::vram::{
    BlockDeallocator, GpuBlockRegistry, VRAM_BUDGET_REMEDIATION, VramBudgeter, VramProbe,
};
use crate::{ForgeError, Result};

pub const DEFAULT_OOM_MAX_RETRIES: u8 = 3;
const CUDA_ERROR_REMEDIATION: &str =
    "Inspect CUDA driver state and kernel logs; non-OOM CUDA failures are not retryable";

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
pub struct OomGuardStats {
    pub oom_intercepts: u64,
    pub batch_reductions: u64,
    pub final_failures: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CudaAllocError {
    MemoryAllocation,
    Other { code: i32, name: String },
}

pub trait CudaMalloc: Send + Sync {
    fn cuda_malloc(&self, size: usize) -> std::result::Result<*mut u8, CudaAllocError>;
}

pub struct OomGuard<'b, P: VramProbe, D: BlockDeallocator, A: CudaMalloc> {
    budgeter: &'b VramBudgeter<P>,
    registry: Arc<Mutex<GpuBlockRegistry<'b, P, D>>>,
    allocator: A,
    min_batch: usize,
    max_retries: u8,
}

impl<'b, P: VramProbe, D: BlockDeallocator, A: CudaMalloc> OomGuard<'b, P, D, A> {
    pub fn new(
        budgeter: &'b VramBudgeter<P>,
        registry: Arc<Mutex<GpuBlockRegistry<'b, P, D>>>,
        allocator: A,
        min_batch: usize,
    ) -> Self {
        Self::with_retries(
            budgeter,
            registry,
            allocator,
            min_batch,
            DEFAULT_OOM_MAX_RETRIES,
        )
    }

    pub fn with_retries(
        budgeter: &'b VramBudgeter<P>,
        registry: Arc<Mutex<GpuBlockRegistry<'b, P, D>>>,
        allocator: A,
        min_batch: usize,
        max_retries: u8,
    ) -> Self {
        Self {
            budgeter,
            registry,
            allocator,
            min_batch: min_batch.max(1),
            max_retries,
        }
    }

    pub fn min_batch(&self) -> usize {
        self.min_batch
    }

    pub fn max_retries(&self) -> u8 {
        self.max_retries
    }

    pub fn alloc_with_retry(&self, size: usize) -> Result<*mut u8> {
        if self.max_retries == 0 {
            return self.allocator.cuda_malloc(size).map_err(|err| match err {
                CudaAllocError::MemoryAllocation => {
                    self.record_oom_intercept(1, 0, 0, size);
                    self.final_budget_failure(format!(
                        "cudaMalloc failed with cudaErrorMemoryAllocation on the single allowed attempt: requested_bytes={size}"
                    ))
                }
                other => self.gpu_error(other),
            });
        }

        for attempt in 1..=self.max_retries {
            match self.allocator.cuda_malloc(size) {
                Ok(ptr) => return Ok(ptr),
                Err(CudaAllocError::MemoryAllocation) => {
                    self.record_oom_intercept(attempt, 0, 0, size);
                    self.evict_after_oom(size, attempt)?;
                    if attempt == self.max_retries {
                        return Err(self.final_budget_failure(format!(
                            "cudaMalloc exhausted OOM guard retries: requested_bytes={size} retries={}",
                            self.max_retries
                        )));
                    }
                }
                Err(other) => return Err(self.gpu_error(other)),
            }
        }

        Err(self.final_budget_failure(format!(
            "cudaMalloc OOM guard reached an unexpected terminal state: requested_bytes={size}"
        )))
    }

    pub fn dispatch_with_retry<F, R>(&self, batch_size: usize, mut f: F) -> Result<R>
    where
        F: FnMut(usize) -> Result<R>,
    {
        let mut batch = batch_size;
        let mut reductions = 0usize;
        loop {
            match f(batch) {
                Ok(output) => return Ok(output),
                Err(err) if err.code() == "CALYX_FORGE_VRAM_BUDGET" => {
                    if reductions >= usize::from(self.max_retries) {
                        return Err(self.final_budget_failure(format!(
                            "dispatch exhausted OOM guard retries: batch_size={batch} retries={}",
                            self.max_retries
                        )));
                    }
                    let Some(next_batch) = self.next_batch(batch) else {
                        return Err(self.final_budget_failure(format!(
                            "dispatch cannot reduce below min_batch={}: batch_size={batch}",
                            self.min_batch
                        )));
                    };
                    reductions += 1;
                    self.budgeter.record_oom_batch_reduction();
                    tracing::warn!(
                        target: "calyx_forge::vram",
                        attempt = reductions,
                        batch_size_before = batch,
                        batch_size_after = next_batch,
                        "CUDA OOM budget error intercepted; reducing batch and retrying"
                    );
                    batch = next_batch;
                }
                Err(err) => return Err(err),
            }
        }
    }

    fn next_batch(&self, batch: usize) -> Option<usize> {
        if batch <= self.min_batch {
            return None;
        }
        let half = batch / 2;
        (half >= self.min_batch && half < batch).then_some(half)
    }

    fn evict_after_oom(&self, size: usize, attempt: u8) -> Result<()> {
        let mut registry = self.registry.lock().map_err(|_| {
            self.final_budget_failure(format!(
                "OOM guard could not lock GPU block registry after cudaMalloc OOM: requested_bytes={size} attempt={attempt}"
            ))
        })?;
        if registry.evict_lru().is_none() {
            return Err(self.final_budget_failure(format!(
                "cudaMalloc OOM and no GPU blocks were evictable: requested_bytes={size} attempt={attempt}"
            )));
        }
        Ok(())
    }

    fn record_oom_intercept(
        &self,
        attempt: u8,
        batch_size_before: usize,
        batch_size_after: usize,
        requested_bytes: usize,
    ) {
        self.budgeter.record_oom_intercept();
        tracing::warn!(
            target: "calyx_forge::vram",
            attempt,
            batch_size_before,
            batch_size_after,
            requested_bytes,
            "CUDA allocation OOM intercepted; evicting and retrying when budget permits"
        );
    }

    fn final_budget_failure(&self, detail: String) -> ForgeError {
        self.budgeter.record_oom_final_failure();
        ForgeError::VramBudget {
            detail,
            remediation: VRAM_BUDGET_REMEDIATION.to_string(),
        }
    }

    fn gpu_error(&self, err: CudaAllocError) -> ForgeError {
        ForgeError::GpuError {
            detail: format!("CUDA allocation failed with non-OOM error: {err:?}"),
            remediation: CUDA_ERROR_REMEDIATION.to_string(),
        }
    }
}

#[cfg(feature = "cuda")]
#[derive(Clone)]
pub struct RawCudaMalloc {
    ctx: std::sync::Arc<crate::cuda::CudaContext>,
}

#[cfg(feature = "cuda")]
impl RawCudaMalloc {
    pub fn new(ctx: std::sync::Arc<crate::cuda::CudaContext>) -> Self {
        Self { ctx }
    }
}

#[cfg(feature = "cuda")]
impl CudaMalloc for RawCudaMalloc {
    fn cuda_malloc(&self, size: usize) -> std::result::Result<*mut u8, CudaAllocError> {
        self.ctx
            .inner()
            .bind_to_thread()
            .map_err(driver_alloc_error)?;
        let ptr =
            unsafe { cudarc::driver::result::malloc_sync(size) }.map_err(driver_alloc_error)?;
        Ok(ptr as usize as *mut u8)
    }
}

#[cfg(feature = "cuda")]
fn driver_alloc_error(err: cudarc::driver::result::DriverError) -> CudaAllocError {
    use cudarc::driver::sys;
    if err.0 == sys::CUresult::CUDA_ERROR_OUT_OF_MEMORY {
        CudaAllocError::MemoryAllocation
    } else {
        CudaAllocError::Other {
            code: err.0 as i32,
            name: format!("{:?}", err.0),
        }
    }
}

#[cfg(test)]
#[path = "oom_guard_tests.rs"]
mod tests;
