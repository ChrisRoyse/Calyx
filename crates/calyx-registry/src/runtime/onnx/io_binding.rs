//! ONNX Runtime I/O binding + provider telemetry for warm lens inference
//! (#1011).
//!
//! For GPU-policy sessions the run path uses `IoBinding`: inputs are bound
//! (host→device transfer happens at bind time on the CUDA EP) and every
//! output is bound to CUDA pinned host memory, so the device→host copy lands
//! in page-locked memory instead of pageable arena buffers. CPU-policy
//! sessions run direct. There is no fallback in either direction: a GPU
//! session that cannot bind or run fails with a structured error.
//!
//! Environment knobs (all logged at session readiness):
//! - `CALYX_ONNX_CUDA_DEVICE` — CUDA device ordinal (default 0; non-integer
//!   values fail closed, and an out-of-range ordinal fails provider
//!   registration at session build because the CUDA dispatch is
//!   `error_on_failure`).
//! - `CALYX_ONNX_IO_BINDING=0` — explicitly disable I/O binding for GPU
//!   sessions (diagnostic; logged, never silent).
//! - `CALYX_ONNX_REQUIRE_STATIC_BINDING=1` — refuse any run whose
//!   (batch, seq) shape differs from the first bound shape instead of
//!   rebinding. This is the CUDA-graph-capture precondition; a dynamic batch
//!   under this mode is a structured error, not a fallback.
//! - `CALYX_ONNX_DISABLE_CPU_EP_FALLBACK=1` — additionally set the ORT
//!   session config that refuses node-level CPU placement at build time.

use std::collections::BTreeSet;

use calyx_core::{CalyxError, Result};
use ort::memory::{AllocationDevice, AllocatorType, MemoryInfo, MemoryType};
use ort::session::{Session, SessionInputValue, SessionOutputs};
use ort::value::Tensor;

use super::{OnnxProviderPolicy, config_invalid};

pub(super) const CUDA_DEVICE_ENV: &str = "CALYX_ONNX_CUDA_DEVICE";
pub(super) const IO_BINDING_ENV: &str = "CALYX_ONNX_IO_BINDING";
pub(super) const REQUIRE_STATIC_BINDING_ENV: &str = "CALYX_ONNX_REQUIRE_STATIC_BINDING";
pub(super) const DISABLE_CPU_EP_FALLBACK_ENV: &str = "CALYX_ONNX_DISABLE_CPU_EP_FALLBACK";

/// Per-runtime run plan: which device, whether I/O binding is active, and the
/// static-shape contract state.
pub(super) struct OnnxRunPlan {
    label: String,
    io_binding: bool,
    device_id: i32,
    require_static: bool,
    bound_shape: Option<(usize, usize)>,
    seen_shapes: BTreeSet<(usize, usize)>,
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

pub(super) fn cpu_ep_fallback_disabled() -> bool {
    env_flag(DISABLE_CPU_EP_FALLBACK_ENV)
}

/// Shared session build for the Calyx-owned ONNX runtimes: device-aware
/// provider registration (fail-loud, no CPU EP in the GPU list) plus the
/// optional ORT-level refusal of node-level CPU placement.
pub(super) fn build_session(
    label: &str,
    model_file: &std::path::Path,
    policy: OnnxProviderPolicy,
) -> Result<Session> {
    let device_id = configured_cuda_device()?;
    let mut builder = Session::builder()
        .map_err(|err| config_invalid(format!("ONNX session builder failed: {err}")))?
        .with_intra_threads(1)
        .map_err(|err| config_invalid(format!("ONNX intra-thread config failed: {err}")))?
        .with_execution_providers(super::fastembed_runtime::execution_providers_on_device(
            policy, device_id,
        ))
        .map_err(|err| {
            config_invalid(format!(
                "ONNX provider config failed for {label} (policy={} device_id={device_id}): {err}",
                policy.as_str()
            ))
        })?;
    if cpu_ep_fallback_disabled() {
        builder = builder
            .with_config_entry("session.disable_cpu_ep_fallback", "1")
            .map_err(|err| {
                config_invalid(format!(
                    "ONNX disable_cpu_ep_fallback config failed for {label}: {err}"
                ))
            })?;
    }
    builder.commit_from_file(model_file).map_err(|err| {
        config_invalid(format!(
            "load ONNX model failed for {label} (policy={} device_id={device_id}): {err}",
            policy.as_str()
        ))
    })
}

fn env_flag(name: &str) -> bool {
    std::env::var(name)
        .map(|raw| {
            let raw = raw.trim();
            raw == "1" || raw.eq_ignore_ascii_case("true")
        })
        .unwrap_or(false)
}

impl OnnxRunPlan {
    /// Build the run plan for a freshly committed session and emit the
    /// readiness telemetry the #1011 acceptance requires: provider selection,
    /// device id, allocator mode, io-binding state, CPU-fallback stance.
    pub(super) fn new(policy: OnnxProviderPolicy, label: impl Into<String>) -> Result<Self> {
        let label = label.into();
        let device_id = configured_cuda_device()?;
        let binding_env_off = std::env::var(IO_BINDING_ENV)
            .map(|raw| {
                let raw = raw.trim();
                raw == "0" || raw.eq_ignore_ascii_case("false")
            })
            .unwrap_or(false);
        let gpu_policy = matches!(policy, OnnxProviderPolicy::CudaFailLoud);
        let io_binding = gpu_policy && !binding_env_off;
        let require_static = env_flag(REQUIRE_STATIC_BINDING_ENV);
        let (allocator, cpu_fallback) = if gpu_policy {
            (
                if io_binding {
                    "cuda_input_bind_pinned_output"
                } else {
                    "ort_default_device_arena"
                },
                "refused_by_provider_list",
            )
        } else {
            ("host", "cpu_explicit_policy")
        };
        eprintln!(
            "CALYX_ONNX_RUNTIME phase=session_ready label={label} provider={} device_id={device_id} io_binding={io_binding} io_binding_env_off={binding_env_off} allocator={allocator} cpu_fallback={cpu_fallback} require_static_binding={require_static} disable_cpu_ep_fallback={}",
            policy.as_str(),
            cpu_ep_fallback_disabled()
        );
        Ok(Self {
            label,
            io_binding,
            device_id,
            require_static,
            bound_shape: None,
            seen_shapes: BTreeSet::new(),
        })
    }

    /// Run the session over named input tensors and hand the outputs to
    /// `extract` before any binding state is torn down.
    pub(super) fn run_extract<R>(
        &mut self,
        session: &mut Session,
        inputs: Vec<(String, Tensor<i64>)>,
        shape: (usize, usize),
        extract: impl FnOnce(&SessionOutputs<'_>) -> Result<R>,
    ) -> Result<R> {
        self.enforce_shape_contract(shape)?;
        if !self.io_binding {
            let named: Vec<(String, SessionInputValue<'_>)> = inputs
                .into_iter()
                .map(|(name, tensor)| (name, SessionInputValue::from(tensor)))
                .collect();
            let outputs = session
                .run(named)
                .map_err(|err| config_invalid(format!("ONNX inference failed: {err}")))?;
            return extract(&outputs);
        }
        let output_names: Vec<String> = session
            .outputs()
            .iter()
            .map(|output| output.name().to_string())
            .collect();
        let mut binding = session.create_binding().map_err(|err| {
            config_invalid(format!(
                "ONNX io-binding create failed for {}: {err}",
                self.label
            ))
        })?;
        // Bind inputs first: the CUDA EP performs the host->device transfer
        // at bind time. The tensors stay alive until run_binding returns.
        for (name, tensor) in &inputs {
            binding.bind_input(name.as_str(), tensor).map_err(|err| {
                config_invalid(format!(
                    "ONNX io-binding bind_input {name} failed for {}: {err}",
                    self.label
                ))
            })?;
        }
        let pinned_output = MemoryInfo::new(
            AllocationDevice::CUDA_PINNED,
            self.device_id,
            AllocatorType::Device,
            MemoryType::CPUOutput,
        )
        .map_err(|err| {
            config_invalid(format!(
                "ONNX io-binding pinned-output MemoryInfo failed for {} device {}: {err}",
                self.label, self.device_id
            ))
        })?;
        for name in &output_names {
            binding
                .bind_output_to_device(name.as_str(), &pinned_output)
                .map_err(|err| {
                    config_invalid(format!(
                        "ONNX io-binding bind_output {name} failed for {}: {err}",
                        self.label
                    ))
                })?;
        }
        let outputs = session.run_binding(&binding).map_err(|err| {
            config_invalid(format!(
                "ONNX io-binding inference failed for {}: {err}",
                self.label
            ))
        })?;
        extract(&outputs)
    }

    fn enforce_shape_contract(&mut self, shape: (usize, usize)) -> Result<()> {
        if self.seen_shapes.insert(shape) {
            eprintln!(
                "CALYX_ONNX_RUNTIME phase=io_binding_shape label={} batch={} seq={} io_binding={} distinct_shapes={}",
                self.label,
                shape.0,
                shape.1,
                self.io_binding,
                self.seen_shapes.len()
            );
        }
        if !self.require_static {
            return Ok(());
        }
        match self.bound_shape {
            None => {
                self.bound_shape = Some(shape);
                Ok(())
            }
            Some(bound) if bound == shape => Ok(()),
            Some(bound) => Err(CalyxError {
                code: "CALYX_ONNX_STATIC_BINDING_SHAPE",
                message: format!(
                    "{} requires the captured static binding shape batch={} seq={} but received batch={} seq={} under {REQUIRE_STATIC_BINDING_ENV}=1",
                    self.label, bound.0, bound.1, shape.0, shape.1
                ),
                remediation: "bucket inputs to the captured shape (fixed batch and sequence length) or unset CALYX_ONNX_REQUIRE_STATIC_BINDING to allow per-shape rebinding",
            }),
        }
    }
}
