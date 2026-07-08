use calyx_core::{CalyxError, Result};
use ort::session::Session;

use super::cpu_fallback_audit::{configured_audit_mode, profiling_file_path};
use super::green_context::GreenContextHandle;
use super::{OnnxProviderPolicy, config_invalid};

pub(super) const CUDA_DEVICE_ENV: &str = "CALYX_ONNX_CUDA_DEVICE";
pub(super) const IO_BINDING_ENV: &str = "CALYX_ONNX_IO_BINDING";
pub(super) const REQUIRE_STATIC_BINDING_ENV: &str = "CALYX_ONNX_REQUIRE_STATIC_BINDING";
pub(super) const DISABLE_CPU_EP_FALLBACK_ENV: &str = "CALYX_ONNX_DISABLE_CPU_EP_FALLBACK";

pub(super) struct ManagedOnnxSession {
    session: Session,
    _green_context: Option<GreenContextHandle>,
}

impl ManagedOnnxSession {
    pub(super) fn as_ref(&self) -> &Session {
        &self.session
    }

    pub(super) fn as_mut(&mut self) -> &mut Session {
        &mut self.session
    }
}

impl std::ops::Deref for ManagedOnnxSession {
    type Target = Session;

    fn deref(&self) -> &Self::Target {
        &self.session
    }
}

impl std::ops::DerefMut for ManagedOnnxSession {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.session
    }
}

/// CUDA device ordinal from the environment; fails closed on garbage input.
pub(super) fn configured_cuda_device() -> Result<i32> {
    let Ok(raw) = std::env::var(CUDA_DEVICE_ENV) else {
        return Ok(0);
    };
    let raw = raw.trim();
    if raw.is_empty() {
        return Ok(0);
    }
    raw.parse::<i32>()
        .ok()
        .filter(|device| *device >= 0)
        .ok_or_else(|| CalyxError {
            code: "CALYX_ONNX_CUDA_DEVICE_INVALID",
            message: format!("{CUDA_DEVICE_ENV}={raw} is not a non-negative CUDA device ordinal"),
            remediation: "set CALYX_ONNX_CUDA_DEVICE to the integer ordinal reported by nvidia-smi, or unset it for device 0",
        })
}

pub(super) fn cpu_ep_fallback_disabled_for_policy(policy: OnnxProviderPolicy) -> Result<bool> {
    let env_requested = env_flag(DISABLE_CPU_EP_FALLBACK_ENV);
    if env_requested && policy == OnnxProviderPolicy::CpuExplicit {
        return Err(CalyxError {
            code: "CALYX_ONNX_CPU_EP_FALLBACK_POLICY_INVALID",
            message: format!(
                "{DISABLE_CPU_EP_FALLBACK_ENV}=1 is invalid for an explicit CPU ONNX session"
            ),
            remediation: "unset CALYX_ONNX_DISABLE_CPU_EP_FALLBACK for CPU-policy sessions, or use CudaFailLoud when every node must stay off CPU",
        });
    }
    Ok(matches!(policy, OnnxProviderPolicy::CudaFailLoud) || env_requested)
}

pub(super) fn configured_cuda_graphs() -> Result<bool> {
    super::cuda_graphs::configured_cuda_graphs()
}

/// Shared session build for Calyx-owned ONNX runtimes: device-aware provider
/// registration plus optional ORT-level refusal of node-level CPU placement.
pub(super) fn build_session(
    label: &str,
    model_file: &std::path::Path,
    policy: OnnxProviderPolicy,
) -> Result<ManagedOnnxSession> {
    let device_id = configured_cuda_device()?;
    let green_context = super::green_context::create(label, policy, device_id)?;
    let compute_stream = green_context.as_ref().map(GreenContextHandle::stream_ptr);
    let mut builder = Session::builder()
        .map_err(|err| config_invalid(format!("ONNX session builder failed: {err}")))?
        .with_intra_threads(1)
        .map_err(|err| config_invalid(format!("ONNX intra-thread config failed: {err}")))?
        .with_execution_providers(
            super::fastembed_runtime::execution_providers_on_device_with_stream(
                policy,
                device_id,
                compute_stream,
            )?,
        )
        .map_err(|err| {
            config_invalid(format!(
                "ONNX provider config failed for {label} (policy={} device_id={device_id}): {err}",
                policy.as_str()
            ))
        })?;
    if cpu_ep_fallback_disabled_for_policy(policy)? {
        builder = builder
            .with_config_entry("session.disable_cpu_ep_fallback", "1")
            .map_err(|err| {
                config_invalid(format!(
                    "ONNX disable_cpu_ep_fallback config failed for {label}: {err}"
                ))
            })?;
    }
    if configured_audit_mode()?.enabled() {
        builder = builder
            .with_profiling(profiling_file_path(label))
            .map_err(|err| {
                config_invalid(format!("ONNX profiling enable failed for {label}: {err}"))
            })?;
    }
    let session = builder.commit_from_file(model_file).map_err(|err| {
        config_invalid(format!(
            "load ONNX model failed for {label} (policy={} device_id={device_id}): {err}",
            policy.as_str()
        ))
    })?;
    Ok(ManagedOnnxSession {
        session,
        _green_context: green_context,
    })
}

pub(super) fn env_flag(name: &str) -> bool {
    std::env::var(name)
        .map(|raw| {
            let raw = raw.trim();
            raw == "1" || raw.eq_ignore_ascii_case("true")
        })
        .unwrap_or(false)
}
