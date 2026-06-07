pub mod context;
pub mod distance;
#[cfg(test)]
mod distance_tests;
pub mod gemm;
pub mod kernels;
pub mod topk;
#[cfg(test)]
mod topk_tests;

use crate::{Backend, DeviceInfo, ForgeError, Result};

pub use context::{CudaContext, init_cuda, query_device_info};
pub use distance::{cosine_batch_gpu, dot_batch_gpu, l2_batch_gpu};
pub use gemm::{bench_gemm_cublas, bench_gemm_reference_cublas, gemm_cublas, probe_allocation};
pub use topk::topk_gpu;

#[derive(Clone, Debug)]
pub struct CudaBackend {
    ctx: CudaContext,
}

impl CudaBackend {
    pub fn new() -> Result<Self> {
        init_cuda(0, false).map(|ctx| Self { ctx })
    }

    pub fn with_context(ctx: CudaContext) -> Self {
        Self { ctx }
    }

    pub fn context(&self) -> &CudaContext {
        &self.ctx
    }
}

impl Backend for CudaBackend {
    fn gemm(
        &self,
        a: &[f32],
        b: &[f32],
        m: usize,
        k: usize,
        n: usize,
        out: &mut [f32],
    ) -> Result<()> {
        gemm::gemm_host(&self.ctx, a, b, m, k, n, out)
    }

    fn cosine(&self, a: &[f32], b: &[f32], dim: usize, out: &mut [f32]) -> Result<()> {
        distance::cosine_host(&self.ctx, a, b, dim, out)
    }

    fn dot(&self, a: &[f32], b: &[f32], dim: usize, out: &mut [f32]) -> Result<()> {
        distance::dot_host(&self.ctx, a, b, dim, out)
    }

    fn l2(&self, a: &[f32], b: &[f32], dim: usize, out: &mut [f32]) -> Result<()> {
        distance::l2_host(&self.ctx, a, b, dim, out)
    }

    fn normalize(&self, _vecs: &mut [f32], _dim: usize) -> Result<()> {
        Err(unimplemented("cuda::normalize"))
    }

    fn topk(&self, scores: &[f32], k: usize) -> Result<Vec<(usize, f32)>> {
        topk::topk_host(&self.ctx, scores, k)
    }

    fn device_info(&self) -> DeviceInfo {
        query_device_info(&self.ctx)
    }
}

fn unimplemented(op: &str) -> ForgeError {
    ForgeError::Unimplemented {
        op: op.to_string(),
        remediation: "Implement the PH13 CUDA kernel card for this operation before enabling it"
            .to_string(),
    }
}

#[cfg(test)]
static CUDA_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
pub(crate) fn test_lock() -> std::sync::MutexGuard<'static, ()> {
    CUDA_TEST_LOCK.lock().unwrap_or_else(|err| err.into_inner())
}
