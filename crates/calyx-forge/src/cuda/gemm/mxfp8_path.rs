use cudarc::driver::CudaSlice;

use super::gemm_cublas;
use crate::cuda::validate::check_device_f32;
use crate::{CudaContext, ForgeError, MXFP8_BLOCK_SIZE, MxFp8Block, Result, decode_mxfp8};

const MXFP8_DEVICE_REMEDIATION: &str =
    "Run the host-dequantized MXFP8 compatibility GEMM with CUDA and cuBLAS available";
const MXFP8_NUMERICAL_REMEDIATION: &str =
    "Reject invalid MXFP8 GEMM dimensions or non-finite outputs before using scores";

/// Compatibility path: dequantizes MXFP8 blocks on the host and runs FP32 cuBLAS.
/// This is not a native FP8 tensor-core GEMM path.
pub fn gemm_mxfp8_fp32_accum(
    ctx: &CudaContext,
    a_blocks: &[MxFp8Block],
    b_blocks: &[MxFp8Block],
    m: usize,
    k: usize,
    n: usize,
    out: &mut CudaSlice<f32>,
) -> Result<()> {
    validate_shapes(a_blocks, b_blocks, m, k, n, out.len())?;
    let stream = ctx.inner().default_stream();
    if m == 0 || n == 0 || k == 0 {
        stream
            .memset_zeros(out)
            .map_err(|err| device_unavailable(ctx, format!("zero MXFP8 output failed: {err}")))?;
        stream
            .synchronize()
            .map_err(|err| device_unavailable(ctx, format!("zero MXFP8 sync failed: {err}")))?;
        return Ok(());
    }

    let a = decode_mxfp8(a_blocks, m * k);
    let b = decode_mxfp8(b_blocks, k * n);
    let a_dev = stream
        .clone_htod(&a)
        .map_err(|err| device_unavailable(ctx, format!("copy decoded MXFP8 A failed: {err}")))?;
    let b_dev = stream
        .clone_htod(&b)
        .map_err(|err| device_unavailable(ctx, format!("copy decoded MXFP8 B failed: {err}")))?;
    gemm_cublas(ctx, &a_dev, &b_dev, m, k, n, out)?;
    check_output_finite(ctx, out)
}

fn validate_shapes(
    a_blocks: &[MxFp8Block],
    b_blocks: &[MxFp8Block],
    m: usize,
    k: usize,
    n: usize,
    out_len: usize,
) -> Result<()> {
    check_len(a_blocks.len(), block_count(m, k)?, "MXFP8 A blocks")?;
    check_len(b_blocks.len(), block_count(k, n)?, "MXFP8 B blocks")?;
    check_len(out_len, checked_mul(m, n, "MXFP8 output")?, "MXFP8 output")?;
    Ok(())
}

fn check_output_finite(ctx: &CudaContext, out: &CudaSlice<f32>) -> Result<()> {
    check_device_f32(
        ctx,
        "gemm_mxfp8_fp32_accum",
        out,
        false,
        MXFP8_NUMERICAL_REMEDIATION,
    )
}

fn block_count(rows: usize, cols: usize) -> Result<usize> {
    Ok(checked_mul(rows, cols, "MXFP8 matrix")?.div_ceil(MXFP8_BLOCK_SIZE))
}

fn checked_mul(rows: usize, cols: usize, name: &str) -> Result<usize> {
    rows.checked_mul(cols)
        .ok_or_else(|| ForgeError::ShapeMismatch {
            expected: vec![rows, cols],
            got: vec![usize::MAX],
            remediation: format!("{name} shape overflows usize"),
        })
}

fn check_len(actual: usize, expected: usize, name: &str) -> Result<()> {
    if actual == expected {
        return Ok(());
    }
    Err(ForgeError::ShapeMismatch {
        expected: vec![expected],
        got: vec![actual],
        remediation: format!("{name} length does not match encoded matrix shape"),
    })
}

fn device_unavailable(ctx: &CudaContext, detail: String) -> ForgeError {
    ForgeError::DeviceUnavailable {
        device: device_label(ctx),
        detail,
        remediation: MXFP8_DEVICE_REMEDIATION.to_string(),
    }
}

fn device_label(ctx: &CudaContext) -> String {
    format!("cuda:{}", ctx.device_idx())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cpu::gemm_f32;
    use crate::mxfp8::encode_mxfp8;

    fn col_major(row: usize, col: usize, rows: usize) -> usize {
        col * rows + row
    }

    fn identity(size: usize) -> Vec<f32> {
        let mut id = vec![0.0; size * size];
        for idx in 0..size {
            id[col_major(idx, idx, size)] = 1.0;
        }
        id
    }

    fn exactish_values(len: usize) -> Vec<f32> {
        (0..len)
            .map(|idx| ((idx % 15) as f32 - 7.0) * 0.125)
            .collect()
    }

    fn within_two_pct(actual: &[f32], expected: &[f32]) -> f32 {
        actual
            .iter()
            .zip(expected.iter())
            .map(|(a, e)| (*a - *e).abs() / e.abs().max(1.0))
            .fold(0.0, f32::max)
    }

    #[test]
    fn gemm_mxfp8_within_2pct() -> Result<()> {
        let _guard = crate::cuda::test_lock();
        let ctx = crate::init_cuda(0, false)?;
        let m = 4;
        let k = 4;
        let n = 4;
        let a = exactish_values(m * k);
        let b = identity(k);
        let a_blocks = encode_mxfp8(&a)?;
        let b_blocks = encode_mxfp8(&b)?;
        let stream = ctx.inner().default_stream();
        let mut out_dev = stream
            .alloc_zeros(m * n)
            .map_err(|err| device_unavailable(&ctx, format!("test output alloc failed: {err}")))?;
        gemm_mxfp8_fp32_accum(&ctx, &a_blocks, &b_blocks, m, k, n, &mut out_dev)?;
        let out = stream
            .clone_dtoh(&out_dev)
            .map_err(|err| device_unavailable(&ctx, format!("test output read failed: {err}")))?;
        let mut expected = vec![0.0; m * n];
        gemm_f32(&a, &b, m, k, n, &mut expected)?;
        let max_rel = within_two_pct(&out, &expected);
        assert!(max_rel <= 0.02, "max_rel={max_rel}");
        println!(
            "gemm_mxfp8_within_2pct PASSED max_rel={max_rel:.6} first={:.6} last={:.6}",
            out[0],
            out[out.len() - 1]
        );
        Ok(())
    }
}
