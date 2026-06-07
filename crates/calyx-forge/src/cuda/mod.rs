pub mod context;
pub mod kernels;

use crate::{Backend, DeviceInfo, ForgeError, Result};

pub use context::{CudaContext, init_cuda, query_device_info};

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
        _a: &[f32],
        _b: &[f32],
        _m: usize,
        _k: usize,
        _n: usize,
        _out: &mut [f32],
    ) -> Result<()> {
        Err(unimplemented("cuda::gemm"))
    }

    fn cosine(&self, _a: &[f32], _b: &[f32], _dim: usize, _out: &mut [f32]) -> Result<()> {
        Err(unimplemented("cuda::cosine"))
    }

    fn dot(&self, _a: &[f32], _b: &[f32], _dim: usize, _out: &mut [f32]) -> Result<()> {
        Err(unimplemented("cuda::dot"))
    }

    fn l2(&self, _a: &[f32], _b: &[f32], _dim: usize, _out: &mut [f32]) -> Result<()> {
        Err(unimplemented("cuda::l2"))
    }

    fn normalize(&self, _vecs: &mut [f32], _dim: usize) -> Result<()> {
        Err(unimplemented("cuda::normalize"))
    }

    fn topk(&self, _scores: &[f32], _k: usize) -> Result<Vec<(usize, f32)>> {
        Err(unimplemented("cuda::topk"))
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
