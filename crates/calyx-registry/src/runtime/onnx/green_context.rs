use calyx_core::{CalyxError, Result};

use super::OnnxProviderPolicy;
#[cfg(feature = "cuda")]
use super::config_invalid;
use super::cuda_graphs::CUDA_GRAPHS_ENV;
#[cfg(feature = "cuda")]
use super::session::CUDA_DEVICE_ENV;

pub(super) const GREEN_CONTEXT_SMS_ENV: &str = "CALYX_ONNX_GREEN_CONTEXT_SMS";

pub(super) fn configured_green_context_sms() -> Result<Option<u32>> {
    let Ok(raw) = std::env::var(GREEN_CONTEXT_SMS_ENV) else {
        return Ok(None);
    };
    let raw = raw.trim();
    if raw.is_empty() {
        return Ok(None);
    }
    raw.parse::<u32>()
        .ok()
        .filter(|value| *value > 0)
        .map(Some)
        .ok_or_else(|| CalyxError {
            code: "CALYX_ONNX_GREEN_CONTEXT_SMS_INVALID",
            message: format!("{GREEN_CONTEXT_SMS_ENV}={raw} is not a positive SM count"),
            remediation: "set CALYX_ONNX_GREEN_CONTEXT_SMS to a positive CUDA SM count, or unset it to use the normal CUDA primary context",
        })
}

pub(super) fn validate_run_plan(policy: OnnxProviderPolicy, cuda_graphs: bool) -> Result<()> {
    let Some(_) = configured_green_context_sms()? else {
        return Ok(());
    };
    if policy != OnnxProviderPolicy::CudaFailLoud {
        return Err(CalyxError {
            code: "CALYX_ONNX_GREEN_CONTEXT_CPU_POLICY",
            message: format!("{GREEN_CONTEXT_SMS_ENV} was requested for a CPU-policy ONNX session"),
            remediation: "enable green contexts only on CudaFailLoud ONNX sessions, or unset CALYX_ONNX_GREEN_CONTEXT_SMS for CPU sessions",
        });
    }
    if cuda_graphs {
        return Err(CalyxError {
            code: "CALYX_ONNX_GREEN_CONTEXT_CUDA_GRAPHS",
            message: format!("{GREEN_CONTEXT_SMS_ENV} cannot be combined with {CUDA_GRAPHS_ENV}=1"),
            remediation: "benchmark green contexts and CUDA Graphs independently; unset one of the two opt-in env vars",
        });
    }
    Ok(())
}

#[cfg(feature = "cuda")]
pub(super) type GreenContextHandle = calyx_forge::CudaGreenContextStream;

#[cfg(not(feature = "cuda"))]
pub(super) struct GreenContextHandle;

#[cfg(not(feature = "cuda"))]
impl GreenContextHandle {
    pub(super) fn stream_ptr(&self) -> *mut () {
        std::ptr::null_mut()
    }
}

#[cfg(feature = "cuda")]
pub(super) fn create(
    label: &str,
    policy: OnnxProviderPolicy,
    device_id: i32,
) -> Result<Option<GreenContextHandle>> {
    let Some(sm_count) = configured_green_context_sms()? else {
        return Ok(None);
    };
    if policy != OnnxProviderPolicy::CudaFailLoud {
        return Err(CalyxError {
            code: "CALYX_ONNX_GREEN_CONTEXT_CPU_POLICY",
            message: format!(
                "{GREEN_CONTEXT_SMS_ENV}={sm_count} was requested for CPU-policy ONNX session {label}"
            ),
            remediation: "enable green contexts only on CudaFailLoud ONNX sessions, or unset CALYX_ONNX_GREEN_CONTEXT_SMS for CPU sessions",
        });
    }
    let device_idx = u32::try_from(device_id).map_err(|_| CalyxError {
        code: "CALYX_ONNX_CUDA_DEVICE_INVALID",
        message: format!("{CUDA_DEVICE_ENV}={device_id} is not a valid CUDA device ordinal"),
        remediation: "set CALYX_ONNX_CUDA_DEVICE to the integer ordinal reported by nvidia-smi, or unset it for device 0",
    })?;
    let stream = calyx_forge::CudaGreenContextStream::create_serving(device_idx, sm_count)
        .map_err(|err| {
            config_invalid(format!(
                "ONNX green context init failed for {label} ({GREEN_CONTEXT_SMS_ENV}={sm_count} device_id={device_id}): {err}"
            ))
        })?;
    eprintln!(
        "CALYX_ONNX_RUNTIME phase=green_context_ready label={label} device_id={device_id} requested_sm_count={} actual_sm_count={} total_sm_count={} green_ctx_id={} workqueue_balanced={}",
        stream.requested_sm_count(),
        stream.actual_sm_count(),
        stream.total_sm_count(),
        stream.green_ctx_id(),
        stream.workqueue_balanced()
    );
    Ok(Some(stream))
}

#[cfg(not(feature = "cuda"))]
pub(super) fn create(
    label: &str,
    _policy: OnnxProviderPolicy,
    _device_id: i32,
) -> Result<Option<GreenContextHandle>> {
    if configured_green_context_sms()?.is_some() {
        return Err(CalyxError {
            code: "CALYX_ONNX_GREEN_CONTEXT_UNSUPPORTED",
            message: format!(
                "{GREEN_CONTEXT_SMS_ENV} was requested for {label}, but this binary was not built with calyx-registry/cuda"
            ),
            remediation: "rebuild with --features cuda, or unset CALYX_ONNX_GREEN_CONTEXT_SMS",
        });
    }
    Ok(None)
}
