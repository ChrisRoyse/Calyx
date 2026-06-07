pub mod distance;
pub mod gemm;
pub mod guard;
pub mod normalize;
pub mod topk;

use crate::{Backend, DeviceInfo, Result};

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

    fn cosine(&self, a: &[f32], b: &[f32], dim: usize, out: &mut [f32]) -> Result<()> {
        distance::cosine_batch(a, b, dim, out)
    }

    fn dot(&self, a: &[f32], b: &[f32], dim: usize, out: &mut [f32]) -> Result<()> {
        distance::dot_batch(a, b, dim, out)
    }

    fn l2(&self, a: &[f32], b: &[f32], dim: usize, out: &mut [f32]) -> Result<()> {
        distance::l2_batch(a, b, dim, out)
    }

    fn normalize(&self, vecs: &mut [f32], dim: usize) -> Result<()> {
        normalize::normalize_f32(vecs, dim)
    }

    fn topk(&self, scores: &[f32], k: usize) -> Result<Vec<(usize, f32)>> {
        topk::topk_f32(scores, k)
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

pub use distance::{cosine_batch, dot_batch, l2_batch};
pub use gemm::gemm_f32;
pub use guard::{check_finite, check_norm_positive, check_shape_2d};
pub use normalize::normalize_f32;
pub use topk::topk_f32;

#[cfg(target_arch = "x86_64")]
fn avx512_available() -> bool {
    std::arch::is_x86_feature_detected!("avx512f")
}

#[cfg(not(target_arch = "x86_64"))]
fn avx512_available() -> bool {
    false
}
