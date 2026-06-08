pub const DISTANCE_PTX: &[u8] = include_bytes!(env!("FORGE_DISTANCE_PTX_PATH"));
pub const TOPK_PTX: &[u8] = include_bytes!(env!("FORGE_TOPK_PTX_PATH"));
pub const MXFP4_GEMM_PTX: &[u8] = include_bytes!(env!("FORGE_MXFP4_GEMM_PTX_PATH"));
pub const DISTANCE_CUBIN: &[u8] = include_bytes!(env!("FORGE_DISTANCE_CUBIN_PATH"));
pub const TOPK_CUBIN: &[u8] = include_bytes!(env!("FORGE_TOPK_CUBIN_PATH"));
pub const MXFP4_GEMM_CUBIN: &[u8] = include_bytes!(env!("FORGE_MXFP4_GEMM_CUBIN_PATH"));

pub const DISTANCE_PTX_PATH: &str = env!("FORGE_DISTANCE_PTX_PATH");
pub const TOPK_PTX_PATH: &str = env!("FORGE_TOPK_PTX_PATH");
pub const MXFP4_GEMM_PTX_PATH: &str = env!("FORGE_MXFP4_GEMM_PTX_PATH");
pub const DISTANCE_CUBIN_PATH: &str = env!("FORGE_DISTANCE_CUBIN_PATH");
pub const TOPK_CUBIN_PATH: &str = env!("FORGE_TOPK_CUBIN_PATH");
pub const MXFP4_GEMM_CUBIN_PATH: &str = env!("FORGE_MXFP4_GEMM_CUBIN_PATH");

#[cfg(test)]
mod tests {
    use super::*;

    const BUILD_RS: &str = include_str!("../../build.rs");

    #[test]
    fn distance_ptx_is_embedded_and_has_header() {
        println!(
            "CUDA_KERNEL distance_ptx_path={} bytes={}",
            DISTANCE_PTX_PATH,
            DISTANCE_PTX.len()
        );
        assert!(!DISTANCE_PTX.is_empty());
        assert!(DISTANCE_PTX.starts_with(b"//\n") || DISTANCE_PTX.starts_with(b".version"));
        assert!(contains_bytes(DISTANCE_PTX, b".version"));
        assert!(contains_bytes(DISTANCE_PTX, b".target sm_120"));
    }

    #[test]
    fn ptx_contains_kernel_entry_points() {
        println!(
            "CUDA_KERNEL topk_ptx_path={} bytes={}",
            TOPK_PTX_PATH,
            TOPK_PTX.len()
        );
        assert!(contains_bytes(DISTANCE_PTX, b"cosine_batch_f32"));
        assert!(contains_bytes(DISTANCE_PTX, b"normalize_rows_f32"));
        assert!(contains_bytes(TOPK_PTX, b"bitonic_topk_f32"));
        assert!(contains_bytes(
            MXFP4_GEMM_PTX,
            b"gemm_mxfp4_fp32_accum_kernel"
        ));
    }

    #[test]
    fn cubin_fast_path_artifacts_are_embedded() {
        println!(
            "CUDA_KERNEL_CUBIN distance={} bytes={} topk={} bytes={} mxfp4={} bytes={}",
            DISTANCE_CUBIN_PATH,
            DISTANCE_CUBIN.len(),
            TOPK_CUBIN_PATH,
            TOPK_CUBIN.len(),
            MXFP4_GEMM_CUBIN_PATH,
            MXFP4_GEMM_CUBIN.len()
        );
        assert!(DISTANCE_CUBIN.len() > 1024);
        assert!(TOPK_CUBIN.len() > 1024);
        assert!(MXFP4_GEMM_CUBIN.len() > 1024);
    }

    #[test]
    fn env_paths_point_to_materialized_out_dir_files() {
        for path in [
            DISTANCE_PTX_PATH,
            TOPK_PTX_PATH,
            DISTANCE_CUBIN_PATH,
            TOPK_CUBIN_PATH,
            MXFP4_GEMM_PTX_PATH,
            MXFP4_GEMM_CUBIN_PATH,
        ] {
            let metadata = std::fs::metadata(path).expect("kernel artifact exists in OUT_DIR");
            println!("CUDA_KERNEL_FILE path={path} bytes={}", metadata.len());
            assert!(metadata.len() > 0);
        }
    }

    #[test]
    fn build_script_uses_explicit_deterministic_math_flags() {
        assert!(!BUILD_RS.contains("--use_fast_math"));
        assert!(BUILD_RS.contains("--ftz=false"));
        assert!(BUILD_RS.contains("--prec-div=true"));
        assert!(BUILD_RS.contains("--prec-sqrt=true"));
        assert!(BUILD_RS.contains("--fmad=false"));
    }

    fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
        haystack
            .windows(needle.len())
            .any(|window| window == needle)
    }
}
