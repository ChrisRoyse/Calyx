pub mod context;
pub mod distance;
#[cfg(test)]
mod distance_tests;
pub mod gemm;
pub mod grouped_gemm;
#[cfg(test)]
mod grouped_gemm_tests;
pub mod kernels;
pub mod ragged_gemm;
pub mod topk;
#[cfg(test)]
mod topk_tests;

use crate::{Backend, DeviceInfo, Result};

pub use crate::mxfp4;
pub use context::{CudaContext, init_cuda, query_device_info};
pub use distance::{cosine_batch_gpu, dot_batch_gpu, l2_batch_gpu, normalize_rows_gpu};
pub use gemm::{
    bench_gemm_cublas, bench_gemm_reference_cublas, gemm_cublas, gemm_mxfp4_fp32_accum,
    gemm_mxfp8_fp32_accum, probe_allocation,
};
pub use grouped_gemm::{
    AbsentSlotSentinel, GemmProblem, GroupedGemmExecutionMode, GroupedGemmPlan,
    build_grouped_gemm_plan, execute_grouped_gemm, execute_grouped_gemm_strict,
    read_grouped_gemm_output,
};
pub use ragged_gemm::{
    RaggedBatch, build_ragged_batch, build_ragged_batch_from_slabs, extract_ragged_results,
    try_extract_ragged_results,
};
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

    pub fn grouped_gemm(&self, plan: &mut GroupedGemmPlan) -> Result<()> {
        grouped_gemm::execute_grouped_gemm(&self.ctx, plan)
    }

    pub fn grouped_gemm_strict(&self, plan: &mut GroupedGemmPlan) -> Result<()> {
        grouped_gemm::execute_grouped_gemm_strict(&self.ctx, plan)
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

    fn normalize(&self, vecs: &mut [f32], dim: usize) -> Result<()> {
        distance::normalize_host(&self.ctx, vecs, dim)
    }

    fn topk(&self, scores: &[f32], k: usize) -> Result<Vec<(usize, f32)>> {
        topk::topk_host(&self.ctx, scores, k)
    }

    fn device_info(&self) -> DeviceInfo {
        query_device_info(&self.ctx)
    }
}

#[cfg(test)]
static CUDA_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
pub(crate) fn test_lock() -> std::sync::MutexGuard<'static, ()> {
    CUDA_TEST_LOCK.lock().unwrap_or_else(|err| err.into_inner())
}
