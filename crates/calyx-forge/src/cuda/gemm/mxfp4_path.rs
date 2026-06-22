use std::str;
use std::sync::Arc;

use cudarc::driver::{CudaModule, CudaSlice, LaunchConfig, PushKernelArg};
use cudarc::nvrtc::Ptx;

use crate::cuda::kernels::MXFP4_GEMM_PTX;
use crate::{CudaContext, ForgeError, MXFP4_BLOCK_SIZE, MXFP4_PACKED_BYTES, MxFp4Block, Result};

const MXFP4_THREADS: u32 = 128;
const MXFP4_DEVICE_REMEDIATION: &str =
    "Run MXFP4 GEMM on Blackwell sm_120 with CUDA 13.3 and embedded Forge kernels";
const MXFP4_NUMERICAL_REMEDIATION: &str =
    "Reject invalid MXFP4 GEMM dimensions or kernel outputs before using scores";

/// Runs the PH15 MXFP4 GEMM path with fp32 accumulation on Blackwell.
///
/// CUDA 13.3 in a manual verification run exposes FP4 storage/conversion headers, but the
/// current cuBLAS C API surface used through `cudarc` does not expose a native
/// FP4 GEMM entry point. The optimized tensor-core promotion path should use
/// CUTLASS 3.x grouped GEMM with an MXFP4 dtype; see NVIDIA CUTLASS
/// `examples/24_gemm_grouped/gemm_grouped.cu`.
pub fn gemm_mxfp4_fp32_accum(
    ctx: &CudaContext,
    a_blocks: &[MxFp4Block],
    b_blocks: &[MxFp4Block],
    m: usize,
    k: usize,
    n: usize,
    out: &mut CudaSlice<f32>,
) -> Result<()> {
    ensure_mxfp4_sm120(ctx.compute_capability(), &device_label(ctx))?;
    validate_shapes(a_blocks, b_blocks, m, k, n, out.len())?;
    let stream = ctx.inner().default_stream();
    if m == 0 || n == 0 || k == 0 {
        stream
            .memset_zeros(out)
            .map_err(|err| device_unavailable(ctx, format!("zero MXFP4 output failed: {err}")))?;
        stream
            .synchronize()
            .map_err(|err| device_unavailable(ctx, format!("zero MXFP4 sync failed: {err}")))?;
        return Ok(());
    }

    let (a_codes, a_scales) = flatten_blocks(a_blocks);
    let (b_codes, b_scales) = flatten_blocks(b_blocks);
    let a_codes_dev = stream
        .clone_htod(&a_codes)
        .map_err(|err| device_unavailable(ctx, format!("copy MXFP4 A codes failed: {err}")))?;
    let a_scales_dev = stream
        .clone_htod(&a_scales)
        .map_err(|err| device_unavailable(ctx, format!("copy MXFP4 A scales failed: {err}")))?;
    let b_codes_dev = stream
        .clone_htod(&b_codes)
        .map_err(|err| device_unavailable(ctx, format!("copy MXFP4 B codes failed: {err}")))?;
    let b_scales_dev = stream
        .clone_htod(&b_scales)
        .map_err(|err| device_unavailable(ctx, format!("copy MXFP4 B scales failed: {err}")))?;

    launch_mxfp4_kernel(
        ctx,
        &a_codes_dev,
        &a_scales_dev,
        &b_codes_dev,
        &b_scales_dev,
        m,
        k,
        n,
        out,
    )?;
    check_output_finite(ctx, out)
}

fn ensure_mxfp4_sm120(compute: (i32, i32), device: &str) -> Result<()> {
    if compute >= (12, 0) {
        return Ok(());
    }
    Err(ForgeError::DeviceUnavailable {
        device: device.to_string(),
        detail: format!(
            "MXFP4 requires sm_120 (Blackwell). Got sm_{}{}",
            compute.0, compute.1
        ),
        remediation: MXFP4_DEVICE_REMEDIATION.to_string(),
    })
}

fn validate_shapes(
    a_blocks: &[MxFp4Block],
    b_blocks: &[MxFp4Block],
    m: usize,
    k: usize,
    n: usize,
    out_len: usize,
) -> Result<()> {
    check_len(a_blocks.len(), block_count(m, k)?, "MXFP4 A blocks")?;
    check_len(b_blocks.len(), block_count(k, n)?, "MXFP4 B blocks")?;
    check_len(out_len, checked_mul(m, n, "MXFP4 output")?, "MXFP4 output")?;
    to_i32(m, "m")?;
    to_i32(k, "k")?;
    to_i32(n, "n")?;
    Ok(())
}

fn flatten_blocks(blocks: &[MxFp4Block]) -> (Vec<u8>, Vec<u8>) {
    let mut codes = Vec::with_capacity(blocks.len() * MXFP4_PACKED_BYTES);
    let mut scales = Vec::with_capacity(blocks.len());
    for block in blocks {
        codes.extend_from_slice(&block.codes);
        scales.push(block.scale_e8m0);
    }
    (codes, scales)
}

#[allow(clippy::too_many_arguments)]
fn launch_mxfp4_kernel(
    ctx: &CudaContext,
    a_codes: &CudaSlice<u8>,
    a_scales: &CudaSlice<u8>,
    b_codes: &CudaSlice<u8>,
    b_scales: &CudaSlice<u8>,
    m: usize,
    k: usize,
    n: usize,
    out: &mut CudaSlice<f32>,
) -> Result<()> {
    let module = mxfp4_module(ctx)?;
    let func = module
        .load_function("gemm_mxfp4_fp32_accum_kernel")
        .map_err(|err| device_unavailable(ctx, format!("load MXFP4 GEMM kernel failed: {err}")))?;
    let cells = checked_mul(m, n, "MXFP4 kernel cells")?;
    let blocks = u32::try_from(cells.div_ceil(MXFP4_THREADS as usize)).map_err(|_| {
        ForgeError::ShapeMismatch {
            expected: vec![u32::MAX as usize],
            got: vec![cells],
            remediation: "MXFP4 GEMM grid exceeds CUDA u32 launch limit".to_string(),
        }
    })?;
    let m_i32 = to_i32(m, "m")?;
    let k_i32 = to_i32(k, "k")?;
    let n_i32 = to_i32(n, "n")?;
    let cfg = LaunchConfig {
        grid_dim: (blocks, 1, 1),
        block_dim: (MXFP4_THREADS, 1, 1),
        shared_mem_bytes: 0,
    };
    let stream = ctx.inner().default_stream();
    let mut launch = stream.launch_builder(&func);
    unsafe {
        launch
            .arg(a_codes)
            .arg(a_scales)
            .arg(b_codes)
            .arg(b_scales)
            .arg(&m_i32)
            .arg(&k_i32)
            .arg(&n_i32)
            .arg(out)
            .launch(cfg)
    }
    .map_err(|err| device_unavailable(ctx, format!("launch MXFP4 GEMM kernel failed: {err}")))?;
    stream
        .synchronize()
        .map_err(|err| device_unavailable(ctx, format!("sync MXFP4 GEMM kernel failed: {err}")))?;
    Ok(())
}

fn mxfp4_module(ctx: &CudaContext) -> Result<Arc<CudaModule>> {
    let ptx = str::from_utf8(MXFP4_GEMM_PTX)
        .map_err(|err| device_unavailable(ctx, format!("MXFP4 GEMM PTX is not UTF-8: {err}")))?;
    ctx.inner()
        .load_module(Ptx::from_src(ptx))
        .map_err(|err| device_unavailable(ctx, format!("MXFP4 GEMM PTX load failed: {err}")))
}

fn check_output_finite(ctx: &CudaContext, out: &CudaSlice<f32>) -> Result<()> {
    let values = ctx
        .inner()
        .default_stream()
        .clone_dtoh(out)
        .map_err(|err| device_unavailable(ctx, format!("read MXFP4 output failed: {err}")))?;
    for (idx, value) in values.iter().enumerate() {
        if !value.is_finite() {
            return Err(ForgeError::NumericalInvariant {
                op: "gemm_mxfp4_fp32_accum".to_string(),
                detail: format!("non-finite output at index {idx}: {value}"),
                remediation: MXFP4_NUMERICAL_REMEDIATION.to_string(),
            });
        }
    }
    Ok(())
}

fn block_count(rows: usize, cols: usize) -> Result<usize> {
    Ok(checked_mul(rows, cols, "MXFP4 matrix")?.div_ceil(MXFP4_BLOCK_SIZE))
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

fn to_i32(value: usize, name: &str) -> Result<i32> {
    i32::try_from(value).map_err(|_| ForgeError::ShapeMismatch {
        expected: vec![i32::MAX as usize],
        got: vec![value],
        remediation: format!("MXFP4 GEMM {name} exceeds i32 kernel argument limit"),
    })
}

fn device_unavailable(ctx: &CudaContext, detail: String) -> ForgeError {
    ForgeError::DeviceUnavailable {
        device: device_label(ctx),
        detail,
        remediation: MXFP4_DEVICE_REMEDIATION.to_string(),
    }
}

fn device_label(ctx: &CudaContext) -> String {
    format!("cuda:{}", ctx.device_idx())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cpu::gemm_f32;
    use crate::mxfp4::encode_mxfp4;

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
            .map(|idx| ((idx % 15) as f32 - 7.0) / 7.0)
            .collect()
    }

    fn within_five_pct(actual: &[f32], expected: &[f32]) -> f32 {
        actual
            .iter()
            .zip(expected.iter())
            .map(|(a, e)| (*a - *e).abs() / e.abs().max(1.0))
            .fold(0.0, f32::max)
    }

    #[test]
    fn sm_check_rejects_pre_blackwell() {
        let err = ensure_mxfp4_sm120((11, 0), "cuda:test")
            .expect_err("pre-Blackwell device must fail closed");
        println!("mxfp4_sm_check PASSED {err}");
        assert!(matches!(err, ForgeError::DeviceUnavailable { .. }));
    }

    #[test]
    fn gemm_mxfp4_within_5pct() -> Result<()> {
        let _guard = crate::cuda::test_lock();
        let ctx = crate::init_cuda(0, false)?;
        let m = 4;
        let k = 4;
        let n = 4;
        let a = exactish_values(m * k);
        let b = identity(k);
        let a_blocks = encode_mxfp4(&a)?;
        let b_blocks = encode_mxfp4(&b)?;
        let stream = ctx.inner().default_stream();
        let mut out_dev = stream
            .alloc_zeros(m * n)
            .map_err(|err| device_unavailable(&ctx, format!("test output alloc failed: {err}")))?;
        gemm_mxfp4_fp32_accum(&ctx, &a_blocks, &b_blocks, m, k, n, &mut out_dev)?;
        let out = stream
            .clone_dtoh(&out_dev)
            .map_err(|err| device_unavailable(&ctx, format!("test output read failed: {err}")))?;
        let mut expected = vec![0.0; m * n];
        gemm_f32(&a, &b, m, k, n, &mut expected)?;
        let max_rel = within_five_pct(&out, &expected);
        assert!(max_rel <= 0.05, "max_rel={max_rel}");
        println!(
            "gemm_mxfp4_within_5pct PASSED max_rel={max_rel:.6} first={:.6} last={:.6}",
            out[0],
            out[out.len() - 1]
        );
        Ok(())
    }
}
