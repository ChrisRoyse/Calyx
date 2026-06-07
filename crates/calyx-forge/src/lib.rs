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
    CURRENT_SEED_VERSION, QuantLevel, QuantizedVec, Quantizer, RotationSeed, SeedId,
    TurboQuantCodec, apply_rotation, apply_rotation_batch, new_seed, seed_id_hex,
};

#[cfg(test)]
mod tests {
    #[test]
    fn crate_metadata_is_present() {
        assert_eq!(env!("CARGO_PKG_NAME"), "calyx-forge");
    }
}
