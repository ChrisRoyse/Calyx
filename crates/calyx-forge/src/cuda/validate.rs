use cudarc::driver::{CudaSlice, LaunchConfig, PushKernelArg};

use crate::cuda::distance::distance_module;
use crate::{CudaContext, ForgeError, Result};

const VALIDATE_THREADS: u32 = 256;
const FLAG_NONFINITE: u32 = 1;
const FLAG_SENTINEL: u32 = 1 << 1;
const FLAG_SENTINEL_RANGE: u32 = 1 << 2;

#[derive(Clone, Copy, Debug)]
pub(crate) struct DeviceRange {
    pub offset: usize,
    pub len: usize,
}

enum RangeMode {
    Finite,
    ExpectedBits(u32),
}

pub(crate) fn audit_output_enabled() -> bool {
    std::env::var("CALYX_FORGE_AUDIT_OUTPUT")
        .map(|value| {
            matches!(
                value.to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}

pub(crate) fn check_device_f32(
    ctx: &CudaContext,
    op: &'static str,
    values: &CudaSlice<f32>,
    sentinel: bool,
    remediation: &'static str,
) -> Result<()> {
    if values.is_empty() {
        return Ok(());
    }
    if audit_output_enabled() {
        let host = read_device(ctx, op, values, remediation)?;
        return scan_values(op, &host, sentinel, remediation);
    }

    let flags = launch_value_validation(ctx, op, values, sentinel, remediation)?;
    decode_flags(op, flags, remediation)
}

pub(crate) fn read_checked_device_f32(
    ctx: &CudaContext,
    op: &'static str,
    values: &CudaSlice<f32>,
    sentinel: bool,
    remediation: &'static str,
) -> Result<Vec<f32>> {
    if audit_output_enabled() {
        let host = read_device(ctx, op, values, remediation)?;
        scan_values(op, &host, sentinel, remediation)?;
        return Ok(host);
    }

    check_device_f32(ctx, op, values, sentinel, remediation)?;
    read_device(ctx, op, values, remediation)
}

pub(crate) fn check_finite_ranges(
    ctx: &CudaContext,
    op: &'static str,
    values: &CudaSlice<f32>,
    ranges: &[DeviceRange],
    remediation: &'static str,
) -> Result<()> {
    if ranges.is_empty() {
        return Ok(());
    }
    if audit_output_enabled() {
        let host = read_device(ctx, op, values, remediation)?;
        return scan_finite_ranges(op, &host, ranges, remediation);
    }
    let flags = launch_range_validation(ctx, op, values, ranges, RangeMode::Finite, remediation)?;
    decode_flags(op, flags, remediation)
}

pub(crate) fn check_sentinel_ranges(
    ctx: &CudaContext,
    op: &'static str,
    values: &CudaSlice<f32>,
    ranges: &[DeviceRange],
    expected_bits: u32,
    remediation: &'static str,
) -> Result<()> {
    if ranges.is_empty() {
        return Ok(());
    }
    if audit_output_enabled() {
        let host = read_device(ctx, op, values, remediation)?;
        return scan_sentinel_ranges(op, &host, ranges, expected_bits, remediation);
    }
    let flags = launch_range_validation(
        ctx,
        op,
        values,
        ranges,
        RangeMode::ExpectedBits(expected_bits),
        remediation,
    )?;
    decode_flags(op, flags, remediation)
}

fn launch_value_validation(
    ctx: &CudaContext,
    op: &'static str,
    values: &CudaSlice<f32>,
    sentinel: bool,
    remediation: &'static str,
) -> Result<u32> {
    let len_i32 = to_i32(values.len(), "validation length", remediation)?;
    let sentinel_i32 = if sentinel { 1 } else { 0 };
    let blocks = grid_blocks(values.len(), remediation)?;
    let stream = ctx.inner().default_stream();
    let mut flags: CudaSlice<u32> = stream.alloc_zeros(1).map_err(|err| {
        device_unavailable(
            ctx,
            format!("{op} flag allocation failed: {err}"),
            remediation,
        )
    })?;
    let module = distance_module(ctx)?;
    let func = ctx
        .cached_function(&module, "distance.validate_f32_flags", "validate_f32_flags")
        .map_err(|err| {
            device_unavailable(
                ctx,
                format!("{op} validation kernel load failed: {err}"),
                remediation,
            )
        })?;
    let cfg = LaunchConfig {
        grid_dim: (blocks, 1, 1),
        block_dim: (VALIDATE_THREADS, 1, 1),
        shared_mem_bytes: 0,
    };
    let mut launch = stream.launch_builder(func.as_ref());
    unsafe {
        launch
            .arg(values)
            .arg(&len_i32)
            .arg(&sentinel_i32)
            .arg(&mut flags)
            .launch(cfg)
    }
    .map_err(|err| {
        device_unavailable(
            ctx,
            format!("{op} validation launch failed: {err}"),
            remediation,
        )
    })?;
    stream.synchronize().map_err(|err| {
        device_unavailable(
            ctx,
            format!("{op} validation sync failed: {err}"),
            remediation,
        )
    })?;
    read_flag(ctx, op, &flags, remediation)
}

fn launch_range_validation(
    ctx: &CudaContext,
    op: &'static str,
    values: &CudaSlice<f32>,
    ranges: &[DeviceRange],
    mode: RangeMode,
    remediation: &'static str,
) -> Result<u32> {
    let (flat, max_len) = flatten_ranges(values.len(), ranges, remediation)?;
    if max_len == 0 {
        return Ok(0);
    }
    let range_count_i32 = to_i32(ranges.len(), "validation range count", remediation)?;
    let range_count_u32 = u32::try_from(ranges.len()).map_err(|_| ForgeError::ShapeMismatch {
        expected: vec![u32::MAX as usize],
        got: vec![ranges.len()],
        remediation: remediation.to_string(),
    })?;
    let (expected_bits, mode_i32) = match mode {
        RangeMode::Finite => (0, 0),
        RangeMode::ExpectedBits(bits) => (bits, 1),
    };
    let stream = ctx.inner().default_stream();
    let ranges_dev = stream.clone_htod(&flat).map_err(|err| {
        device_unavailable(ctx, format!("{op} range upload failed: {err}"), remediation)
    })?;
    let mut flags: CudaSlice<u32> = stream.alloc_zeros(1).map_err(|err| {
        device_unavailable(
            ctx,
            format!("{op} range flag allocation failed: {err}"),
            remediation,
        )
    })?;
    let module = distance_module(ctx)?;
    let func = ctx
        .cached_function(
            &module,
            "distance.validate_f32_ranges_flags",
            "validate_f32_ranges_flags",
        )
        .map_err(|err| {
            device_unavailable(
                ctx,
                format!("{op} range validation load failed: {err}"),
                remediation,
            )
        })?;
    let cfg = LaunchConfig {
        grid_dim: (grid_blocks(max_len, remediation)?, range_count_u32, 1),
        block_dim: (VALIDATE_THREADS, 1, 1),
        shared_mem_bytes: 0,
    };
    let mut launch = stream.launch_builder(func.as_ref());
    unsafe {
        launch
            .arg(values)
            .arg(&ranges_dev)
            .arg(&range_count_i32)
            .arg(&expected_bits)
            .arg(&mode_i32)
            .arg(&mut flags)
            .launch(cfg)
    }
    .map_err(|err| {
        device_unavailable(
            ctx,
            format!("{op} range validation launch failed: {err}"),
            remediation,
        )
    })?;
    stream.synchronize().map_err(|err| {
        device_unavailable(
            ctx,
            format!("{op} range validation sync failed: {err}"),
            remediation,
        )
    })?;
    read_flag(ctx, op, &flags, remediation)
}

fn read_flag(
    ctx: &CudaContext,
    op: &'static str,
    flags: &CudaSlice<u32>,
    remediation: &'static str,
) -> Result<u32> {
    let values = ctx
        .inner()
        .default_stream()
        .clone_dtoh(flags)
        .map_err(|err| {
            device_unavailable(
                ctx,
                format!("{op} flag readback failed: {err}"),
                remediation,
            )
        })?;
    Ok(values.first().copied().unwrap_or(0))
}

fn read_device(
    ctx: &CudaContext,
    op: &'static str,
    values: &CudaSlice<f32>,
    remediation: &'static str,
) -> Result<Vec<f32>> {
    ctx.inner()
        .default_stream()
        .clone_dtoh(values)
        .map_err(|err| {
            device_unavailable(
                ctx,
                format!("{op} output readback failed: {err}"),
                remediation,
            )
        })
}

fn scan_values(
    op: &'static str,
    values: &[f32],
    sentinel: bool,
    remediation: &'static str,
) -> Result<()> {
    for (idx, value) in values.iter().enumerate() {
        if sentinel && *value <= -1.5 {
            return Err(numerical(
                op,
                format!("zero-norm query or candidate at index {idx}"),
                remediation,
            ));
        }
        if !value.is_finite() {
            return Err(numerical(
                op,
                format!("non-finite output at index {idx}: {value}"),
                remediation,
            ));
        }
    }
    Ok(())
}

fn scan_finite_ranges(
    op: &'static str,
    values: &[f32],
    ranges: &[DeviceRange],
    remediation: &'static str,
) -> Result<()> {
    for range in ranges {
        let end = checked_end(range.offset, range.len, values.len(), remediation)?;
        for (rel_idx, value) in values[range.offset..end].iter().enumerate() {
            if !value.is_finite() {
                return Err(numerical(
                    op,
                    format!(
                        "non-finite ranged output at offset {} relative index {}: {}",
                        range.offset, rel_idx, value
                    ),
                    remediation,
                ));
            }
        }
    }
    Ok(())
}

fn scan_sentinel_ranges(
    op: &'static str,
    values: &[f32],
    ranges: &[DeviceRange],
    expected_bits: u32,
    remediation: &'static str,
) -> Result<()> {
    for range in ranges {
        let end = checked_end(range.offset, range.len, values.len(), remediation)?;
        for (rel_idx, value) in values[range.offset..end].iter().enumerate() {
            if value.to_bits() != expected_bits {
                return Err(numerical(
                    op,
                    format!(
                        "sentinel range overwritten at offset {} relative index {}",
                        range.offset, rel_idx
                    ),
                    remediation,
                ));
            }
        }
    }
    Ok(())
}

fn flatten_ranges(
    total_len: usize,
    ranges: &[DeviceRange],
    remediation: &'static str,
) -> Result<(Vec<i32>, usize)> {
    let mut flat = Vec::with_capacity(ranges.len() * 2);
    let mut max_len = 0;
    for range in ranges {
        checked_end(range.offset, range.len, total_len, remediation)?;
        flat.push(to_i32(
            range.offset,
            "validation range offset",
            remediation,
        )?);
        flat.push(to_i32(range.len, "validation range length", remediation)?);
        max_len = max_len.max(range.len);
    }
    Ok((flat, max_len))
}

fn checked_end(
    offset: usize,
    len: usize,
    total_len: usize,
    remediation: &'static str,
) -> Result<usize> {
    let end = offset
        .checked_add(len)
        .ok_or_else(|| ForgeError::ShapeMismatch {
            expected: vec![usize::MAX],
            got: vec![offset, len],
            remediation: remediation.to_string(),
        })?;
    if end <= total_len {
        Ok(end)
    } else {
        Err(ForgeError::ShapeMismatch {
            expected: vec![total_len],
            got: vec![offset, len],
            remediation: remediation.to_string(),
        })
    }
}

fn grid_blocks(len: usize, remediation: &'static str) -> Result<u32> {
    u32::try_from(len.div_ceil(VALIDATE_THREADS as usize)).map_err(|_| ForgeError::ShapeMismatch {
        expected: vec![u32::MAX as usize],
        got: vec![len],
        remediation: remediation.to_string(),
    })
}

fn to_i32(value: usize, name: &str, remediation: &'static str) -> Result<i32> {
    i32::try_from(value).map_err(|_| ForgeError::ShapeMismatch {
        expected: vec![i32::MAX as usize],
        got: vec![value],
        remediation: format!("{name} exceeds CUDA i32 limit; {remediation}"),
    })
}

fn decode_flags(op: &'static str, flags: u32, remediation: &'static str) -> Result<()> {
    if flags & FLAG_SENTINEL != 0 {
        return Err(numerical(
            op,
            "device validation flag reported zero-norm sentinel output".to_string(),
            remediation,
        ));
    }
    if flags & FLAG_SENTINEL_RANGE != 0 {
        return Err(numerical(
            op,
            "device validation flag reported absent sentinel overwrite".to_string(),
            remediation,
        ));
    }
    if flags & FLAG_NONFINITE != 0 {
        return Err(numerical(
            op,
            "device validation flag reported non-finite output".to_string(),
            remediation,
        ));
    }
    Ok(())
}

fn numerical(op: &'static str, detail: String, remediation: &'static str) -> ForgeError {
    ForgeError::NumericalInvariant {
        op: op.to_string(),
        detail,
        remediation: remediation.to_string(),
    }
}

fn device_unavailable(ctx: &CudaContext, detail: String, remediation: &'static str) -> ForgeError {
    ForgeError::DeviceUnavailable {
        device: format!("cuda:{}", ctx.device_idx()),
        detail,
        remediation: remediation.to_string(),
    }
}
