pub mod gemm;

use crate::{Backend, DeviceInfo, ForgeError, Result};

#[derive(Clone, Debug)]
pub struct CpuBackend {
    avx512: bool,
}

impl CpuBackend {
    pub fn new() -> Self {
        let avx512 = avx512_available();
        if !avx512 {
            tracing::warn!(
                "CALYX_FORGE_CPU_AVX512_UNAVAILABLE falling back to f32x8-compatible path"
            );
        }
        Self { avx512 }
    }

    pub fn avx512_available(&self) -> bool {
        self.avx512
    }

    pub fn simd_path(&self) -> &'static str {
        if self.avx512 { "f32x16" } else { "f32x8" }
    }
}

impl Default for CpuBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl Backend for CpuBackend {
    fn gemm(
        &self,
        a: &[f32],
        b: &[f32],
        m: usize,
        k: usize,
        n: usize,
        out: &mut [f32],
    ) -> Result<()> {
        gemm::gemm_f32(a, b, m, k, n, out)
    }

    fn cosine(&self, _a: &[f32], _b: &[f32], _dim: usize, _out: &mut [f32]) -> Result<()> {
        Err(unimplemented_op("cpu.cosine"))
    }

    fn dot(&self, _a: &[f32], _b: &[f32], _dim: usize, _out: &mut [f32]) -> Result<()> {
        Err(unimplemented_op("cpu.dot"))
    }

    fn l2(&self, _a: &[f32], _b: &[f32], _dim: usize, _out: &mut [f32]) -> Result<()> {
        Err(unimplemented_op("cpu.l2"))
    }

    fn normalize(&self, _vecs: &mut [f32], _dim: usize) -> Result<()> {
        Err(unimplemented_op("cpu.normalize"))
    }

    fn topk(&self, _scores: &[f32], _k: usize) -> Result<Vec<(usize, f32)>> {
        Err(unimplemented_op("cpu.topk"))
    }

    fn device_info(&self) -> DeviceInfo {
        DeviceInfo {
            kind: crate::BackendKind::Cpu,
            name: "calyx-cpu".to_string(),
            avx512: self.avx512,
            vram_mib: None,
        }
    }
}

pub use gemm::gemm_f32;

fn unimplemented_op(op: &str) -> ForgeError {
    ForgeError::Unimplemented {
        op: op.to_string(),
        remediation: "implement the PH12 kernel before dispatching this op".to_string(),
    }
}

#[cfg(target_arch = "x86_64")]
fn avx512_available() -> bool {
    std::arch::is_x86_feature_detected!("avx512f")
}

#[cfg(not(target_arch = "x86_64"))]
fn avx512_available() -> bool {
    false
}
