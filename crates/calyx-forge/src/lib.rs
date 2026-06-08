//! Forge math runtime skeleton for CPU, CUDA, and quantized kernels.

pub mod autotune;
mod backend;
pub mod cpu;
#[cfg(feature = "cuda")]
pub mod cuda;
mod error;
#[path = "cuda/mxfp4.rs"]
pub mod mxfp4;
#[path = "cuda/mxfp8.rs"]
pub mod mxfp8;
pub mod quant;

pub use autotune::{
    AbHook, AutotuneCache, AutotuneKey, BenchCudaContext, BenchResult, EPSILON, Explorer,
    ExplorerPolicy, MIN_PROMOTE_MARGIN, MIN_PROMOTE_TRIALS, PromotionAction, PromotionEvent,
    autotune, log_promotion, microbench, next_candidate, promote_if_winner, record_trial,
    rollback_promotion, should_promote, should_use_challenger,
};
pub use backend::{
    Backend, BackendKind, BestConfig, CUDA_EXACT_TOPK_MAX_K, DeviceInfo,
    FORGE_DEFERRED_BACKEND_OPS, FORGE_SHIPPED_BACKEND_OPS, Result,
};
pub use cpu::CpuBackend;
#[cfg(feature = "cuda")]
pub use cuda::{
    AbsentSlotSentinel, CudaBackend, CudaContext, GemmProblem, GroupedGemmExecutionMode,
    GroupedGemmPlan, RaggedBatch, build_grouped_gemm_plan, build_ragged_batch,
    build_ragged_batch_from_slabs, execute_grouped_gemm, execute_grouped_gemm_strict,
    extract_ragged_results, init_cuda, query_device_info, read_grouped_gemm_output,
    try_extract_ragged_results,
};
pub use error::ForgeError;
pub use mxfp4::{
    MXFP4_BLOCK_SIZE, MXFP4_PACKED_BYTES, MxFp4Block, decode_mxfp4, decode_mxfp4_block, e8m0_scale,
    encode_mxfp4, encode_mxfp4_block,
};
pub use mxfp8::{
    MXFP8_BLOCK_BYTES, MXFP8_BLOCK_SIZE, MxFp8Block, decode_mxfp8, decode_mxfp8_block,
    encode_mxfp8, encode_mxfp8_block,
};
pub use quant::{
    AssayQuantSafety, BinaryCodec, CURRENT_SEED_VERSION, MxFp4Codec, QjlResidual, QuantLevel,
    QuantizedVec, Quantizer, RotationSeed, SeedId, TurboQuantCodec, apply_inverse_rotation,
    apply_rotation, apply_rotation_batch, binary_prefilter, dot_estimate_unbiased,
    dot_qjl_correction, encode_qjl_residual, hamming_dot_estimate, new_seed, seed_id_hex,
};

#[cfg(test)]
mod tests {
    #[test]
    fn crate_metadata_is_present() {
        assert_eq!(env!("CARGO_PKG_NAME"), "calyx-forge");
    }
}
