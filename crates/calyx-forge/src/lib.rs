//! Forge math runtime skeleton for CPU, CUDA, and quantized kernels.

mod backend;
pub mod cpu;
#[cfg(feature = "cuda")]
pub mod cuda;
mod error;
#[path = "cuda/mxfp4.rs"]
pub mod mxfp4;
pub mod quant;

pub use backend::{Backend, BackendKind, BestConfig, DeviceInfo, Result};
pub use cpu::CpuBackend;
#[cfg(feature = "cuda")]
pub use cuda::{CudaBackend, CudaContext, init_cuda, query_device_info};
pub use error::ForgeError;
pub use mxfp4::{
    MXFP4_BLOCK_SIZE, MXFP4_PACKED_BYTES, MxFp4Block, decode_mxfp4, decode_mxfp4_block, e8m0_scale,
    encode_mxfp4, encode_mxfp4_block,
};
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
