//! CUDA BFC device-arena environment knobs (#1143). All fail closed on
//! garbage input; see `io_binding` for how they attach to sessions and runs.

use std::fs;
use std::path::Path;

use calyx_core::{CalyxError, Result};

use super::{OnnxProviderPolicy, config_invalid};

pub(super) const GPU_MEM_LIMIT_ENV: &str = "CALYX_ONNX_GPU_MEM_LIMIT_MIB";
pub(super) const ARENA_SHRINK_ENV: &str = "CALYX_ONNX_ARENA_SHRINK";
pub(super) const MAX_DISTINCT_SHAPES_ENV: &str = "CALYX_ONNX_MAX_DISTINCT_SHAPES";

const DEFAULT_MAX_DISTINCT_SHAPES: usize = 64;
pub(super) const ARENA_SHRINKAGE_RUN_KEY: &str = "memory.enable_memory_arena_shrinkage";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ArenaShrinkPolicy {
    Off,
    NewShape,
    Always,
}

impl ArenaShrinkPolicy {
    pub(super) const fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::NewShape => "new-shape",
            Self::Always => "always",
        }
    }
}

/// Optional hard cap (bytes) on the CUDA BFC device arena.
pub(super) fn configured_gpu_mem_limit() -> Result<Option<usize>> {
    let Ok(raw) = std::env::var(GPU_MEM_LIMIT_ENV) else {
        return Ok(None);
    };
    let raw = raw.trim();
    if raw.is_empty() {
        return Ok(None);
    }
    raw.parse::<usize>()
        .ok()
        .filter(|mib| *mib > 0)
        .and_then(|mib| mib.checked_mul(1024 * 1024))
        .map(Some)
        .ok_or_else(|| CalyxError {
            code: "CALYX_ONNX_GPU_MEM_LIMIT_INVALID",
            message: format!("{GPU_MEM_LIMIT_ENV}={raw} is not a positive MiB count"),
            remediation: "set CALYX_ONNX_GPU_MEM_LIMIT_MIB to a positive integer MiB budget for the CUDA arena, or unset it for no cap",
        })
}

pub(super) fn preflight_gpu_mem_limit_for_artifacts<'a>(
    label: &str,
    policy: OnnxProviderPolicy,
    artifacts: impl IntoIterator<Item = &'a Path>,
) -> Result<()> {
    if policy != OnnxProviderPolicy::CudaFailLoud {
        return Ok(());
    }
    let Some(limit) = configured_gpu_mem_limit()? else {
        return Ok(());
    };
    let mut total = 0_u64;
    for path in artifacts {
        let bytes = fs::metadata(path)
            .map_err(|err| {
                config_invalid(format!(
                    "read ONNX artifact metadata for {} failed: {err}",
                    path.display()
                ))
            })?
            .len();
        total = total.saturating_add(bytes);
    }
    if total > limit as u64 {
        return Err(config_invalid(format!(
            "{label} refused before ONNX session init: {GPU_MEM_LIMIT_ENV}={} MiB caps the CUDA BFC arena below the resolved artifact bytes {} MiB; refusing before ORT partial initialization can corrupt process teardown",
            limit / (1024 * 1024),
            total.div_ceil(1024 * 1024)
        )));
    }
    Ok(())
}

pub(super) fn configured_arena_shrink() -> Result<ArenaShrinkPolicy> {
    let Ok(raw) = std::env::var(ARENA_SHRINK_ENV) else {
        return Ok(ArenaShrinkPolicy::NewShape);
    };
    match raw.trim() {
        "" => Ok(ArenaShrinkPolicy::NewShape),
        "off" => Ok(ArenaShrinkPolicy::Off),
        "new-shape" => Ok(ArenaShrinkPolicy::NewShape),
        "always" => Ok(ArenaShrinkPolicy::Always),
        other => Err(CalyxError {
            code: "CALYX_ONNX_ARENA_SHRINK_INVALID",
            message: format!("{ARENA_SHRINK_ENV}={other} is not a known arena shrink policy"),
            remediation: "set CALYX_ONNX_ARENA_SHRINK to off, new-shape, or always (default new-shape)",
        }),
    }
}

pub(super) fn configured_max_distinct_shapes() -> Result<usize> {
    let Ok(raw) = std::env::var(MAX_DISTINCT_SHAPES_ENV) else {
        return Ok(DEFAULT_MAX_DISTINCT_SHAPES);
    };
    let raw = raw.trim();
    if raw.is_empty() {
        return Ok(DEFAULT_MAX_DISTINCT_SHAPES);
    }
    raw.parse::<usize>()
        .ok()
        .filter(|limit| *limit > 0)
        .ok_or_else(|| CalyxError {
            code: "CALYX_ONNX_SHAPE_LIMIT_INVALID",
            message: format!("{MAX_DISTINCT_SHAPES_ENV}={raw} is not a positive shape count"),
            remediation: "set CALYX_ONNX_MAX_DISTINCT_SHAPES to a positive integer (default 64), or unset it",
        })
}
