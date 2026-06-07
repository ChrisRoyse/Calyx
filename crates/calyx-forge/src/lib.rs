//! Forge math runtime skeleton for CPU, CUDA, and quantized kernels.

mod backend;
pub mod cpu;
#[cfg(feature = "cuda")]
pub mod cuda;
mod error;
pub mod quant;

pub use backend::{Backend, BackendKind, BestConfig, DeviceInfo, Result};
pub use cpu::CpuBackend;
#[cfg(feature = "cuda")]
pub use cuda::{CudaBackend, CudaContext, init_cuda, query_device_info};
pub use error::ForgeError;
pub use quant::{
    BinaryCodec, CURRENT_SEED_VERSION, QjlResidual, QuantLevel, QuantizedVec, Quantizer,
    RotationSeed, SeedId, TurboQuantCodec, apply_inverse_rotation, apply_rotation,
    apply_rotation_batch, binary_prefilter, dot_estimate_unbiased, dot_qjl_correction,
    encode_qjl_residual, hamming_dot_estimate, new_seed, seed_id_hex,
};

#[cfg(test)]
mod tests {
    #[test]
    fn crate_metadata_is_present() {
        assert_eq!(env!("CARGO_PKG_NAME"), "calyx-forge");
    }
}
