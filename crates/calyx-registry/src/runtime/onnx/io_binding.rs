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
//! - `CALYX_ONNX_CUDA_GRAPHS=1` — enable ORT CUDA Graph capture/replay for
//!   GPU-policy sessions. This requires I/O binding, and assigns a stable
//!   `gpu_graph_id` per observed `(batch, seq)` shape. Invalid values or an
//!   incompatible run plan fail closed. Graph mode disables the default arena
//!   shrink run option; an explicit non-`off` arena-shrink policy is refused.
//! - `CALYX_ONNX_GREEN_CONTEXT_SMS=<n>` — opt a GPU-policy session into a CUDA
//!   green-context user stream with an SM slice of at least `n` SMs and balanced
//!   work queues. Invalid values, CPU-policy sessions, unsupported builds, or
//!   `CALYX_ONNX_CUDA_GRAPHS=1` fail closed.
//! - `CALYX_ONNX_DISABLE_CPU_EP_FALLBACK=1` — additionally set the ORT
//!   session config that refuses node-level CPU placement at build time.
//!
//! Device-arena controls (#1143 — BFC arena growth across dynamic shapes):
//! - `CALYX_ONNX_GPU_MEM_LIMIT_MIB` — hard cap (MiB) on the CUDA BFC arena;
//!   exhaustion becomes a structured error at a defined budget instead of
//!   eating the device from co-tenants.
//! - `CALYX_ONNX_ARENA_SHRINK` — `off` | `new-shape` (default) | `always`:
//!   when to request `memory.enable_memory_arena_shrinkage` for the run.
//! - `CALYX_ONNX_MAX_DISTINCT_SHAPES` — fail-loud cap (default 64) on the
//!   distinct (batch, seq) shapes a GPU session may run; batch/seq bucketing
//!   keeps real workloads far below it, so reaching it means a caller
//!   regressed into unbounded shape diversity.
//! - `CALYX_ONNX_CPU_FALLBACK_AUDIT` — `off` | `warn` | `fail` (#1142): parse
//!   the ORT profiling trace after the first run and surface / refuse a
//!   GPU-policy session that runs too many compute nodes on CPU (int8 graphs
//!   have no CUDA kernels). See `cpu_fallback_audit`.

use std::collections::BTreeSet;

use calyx_core::{CalyxError, Result};
use ort::memory::{AllocationDevice, AllocatorType, MemoryInfo, MemoryType};
use ort::session::{RunOptions, Session, SessionInputValue, SessionOutputs};
use ort::value::Tensor;

use super::arena::{
    ARENA_SHRINKAGE_RUN_KEY, ArenaShrinkPolicy, MAX_DISTINCT_SHAPES_ENV, configured_arena_shrink,
    configured_gpu_mem_limit, configured_max_distinct_shapes,
};
use super::cpu_fallback_audit::{
    AuditMode, audit_from_trace, configured_audit_mode, configured_max_cpu_fraction,
};
use super::cuda_graphs::{CUDA_GRAPHS_ENV, CudaGraphRunConfig, CudaGraphRunRequest};
use super::session::{
    IO_BINDING_ENV, REQUIRE_STATIC_BINDING_ENV, configured_cuda_device, configured_cuda_graphs,
    cpu_ep_fallback_disabled, env_flag,
};
use super::{OnnxProviderPolicy, config_invalid};

/// Per-runtime run plan: which device, whether I/O binding is active, and the
/// static-shape contract state.
#[derive(Debug)]
pub(super) struct OnnxRunPlan {
    label: String,
    io_binding: bool,
    gpu_policy: bool,
    device_id: i32,
    require_static: bool,
    cuda_graphs: CudaGraphRunConfig,
    arena_shrink: ArenaShrinkPolicy,
    max_distinct_shapes: usize,
    audit_mode: AuditMode,
    max_cpu_fraction: f64,
    audited: bool,
    bound_shape: Option<(usize, usize)>,
    seen_shapes: BTreeSet<(usize, usize)>,
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
        let cuda_graphs = configured_cuda_graphs()?;
        let green_context_sms = super::green_context::configured_green_context_sms()?;
        super::green_context::validate_run_plan(policy, cuda_graphs)?;
        if cuda_graphs && !gpu_policy {
            return Err(CalyxError {
                code: "CALYX_ONNX_CUDA_GRAPHS_CPU_POLICY",
                message: format!(
                    "{CUDA_GRAPHS_ENV}=1 was requested for CPU-policy ONNX session {label}"
                ),
                remediation: "enable CUDA graphs only on CudaFailLoud sessions, or unset CALYX_ONNX_CUDA_GRAPHS for CPU sessions",
            });
        }
        if cuda_graphs && !io_binding {
            return Err(CalyxError {
                code: "CALYX_ONNX_CUDA_GRAPHS_IO_BINDING",
                message: format!(
                    "{CUDA_GRAPHS_ENV}=1 requires I/O binding for {label}, but {IO_BINDING_ENV} disabled it"
                ),
                remediation: "unset CALYX_ONNX_IO_BINDING or set it to 1 before enabling CUDA graphs",
            });
        }
        let arena_shrink =
            super::cuda_graphs::compatible_arena_shrink(cuda_graphs, configured_arena_shrink()?)?;
        let max_distinct_shapes = configured_max_distinct_shapes()?;
        let mem_limit = configured_gpu_mem_limit()?;
        let audit_mode = configured_audit_mode()?;
        let max_cpu_fraction = configured_max_cpu_fraction()?;
        let (allocator, cpu_fallback) = if gpu_policy {
            (
                if cuda_graphs {
                    "cuda_graph_static_device_io"
                } else if io_binding {
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
            "CALYX_ONNX_RUNTIME phase=session_ready label={label} provider={} device_id={device_id} io_binding={io_binding} io_binding_env_off={binding_env_off} allocator={allocator} cpu_fallback={cpu_fallback} require_static_binding={require_static} cuda_graphs={cuda_graphs} green_context_sms={} disable_cpu_ep_fallback={} arena_extend=same_as_requested gpu_mem_limit_mib={} arena_shrink={} max_distinct_shapes={max_distinct_shapes} cpu_fallback_audit={} max_cpu_node_fraction={max_cpu_fraction:.4}",
            policy.as_str(),
            green_context_sms
                .map(|count| count.to_string())
                .unwrap_or_else(|| "off".to_string()),
            cpu_ep_fallback_disabled(),
            mem_limit
                .map(|bytes| (bytes / (1024 * 1024)).to_string())
                .unwrap_or_else(|| "none".to_string()),
            arena_shrink.as_str(),
            audit_mode.as_str()
        );
        Ok(Self {
            label,
            io_binding,
            gpu_policy,
            device_id,
            require_static,
            cuda_graphs: CudaGraphRunConfig::new(cuda_graphs),
            arena_shrink,
            max_distinct_shapes,
            audit_mode,
            max_cpu_fraction,
            audited: false,
            bound_shape: None,
            seen_shapes: BTreeSet::new(),
        })
    }

    /// GPU sessions pad token batches to stable power-of-two buckets so the
    /// distinct-shape set the CUDA arena retains allocations for stays
    /// bounded (#1143). CPU sessions run exact batches.
    pub(super) const fn pads_batches(&self) -> bool {
        self.gpu_policy
    }

    /// Arena shrinkage request for this run, per policy: reclaim the device
    /// arena's transient over-extension after first-seen shapes (`new-shape`),
    /// after every run (`always`), or never (`off`). Logged whenever active.
    fn run_options(
        &mut self,
        shape: (usize, usize),
        new_shape: bool,
    ) -> Result<Option<RunOptions>> {
        let shrink = self.gpu_policy
            && match self.arena_shrink {
                ArenaShrinkPolicy::Off => false,
                ArenaShrinkPolicy::NewShape => new_shape,
                ArenaShrinkPolicy::Always => true,
            };
        if !shrink && !self.cuda_graphs.enabled() {
            return Ok(None);
        }
        let mut options = RunOptions::new().map_err(|err| {
            config_invalid(format!(
                "ONNX RunOptions create failed for {}: {err}",
                self.label
            ))
        })?;
        if shrink {
            options
                .add_config_entry(ARENA_SHRINKAGE_RUN_KEY, format!("gpu:{}", self.device_id))
                .map_err(|err| {
                    config_invalid(format!(
                        "ONNX arena shrinkage config failed for {}: {err}",
                        self.label
                    ))
                })?;
            eprintln!(
                "CALYX_ONNX_RUNTIME phase=arena_shrink label={} device_id={} policy={} distinct_shapes={}",
                self.label,
                self.device_id,
                self.arena_shrink.as_str(),
                self.seen_shapes.len()
            );
        }
        self.cuda_graphs
            .add_run_options(&mut options, &self.label, shape, new_shape)?;
        Ok(Some(options))
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
        let new_shape = self.enforce_shape_contract(shape)?;
        let run_options = self.run_options(shape, new_shape)?;
        if !self.io_binding {
            let named: Vec<(String, SessionInputValue<'_>)> = inputs
                .into_iter()
                .map(|(name, tensor)| (name, SessionInputValue::from(tensor)))
                .collect();
            let outputs = match &run_options {
                Some(options) => session.run_with_options(named, options),
                None => session.run(named),
            }
            .map_err(|err| config_invalid(format!("ONNX inference failed: {err}")))?;
            let result = extract(&outputs)?;
            drop(outputs);
            self.audit_placement_once(session)?;
            return Ok(result);
        }
        let output_names: Vec<String> = session
            .outputs()
            .iter()
            .map(|output| output.name().to_string())
            .collect();
        if self.cuda_graphs.enabled() {
            let result = self.cuda_graphs.run_extract(
                session,
                CudaGraphRunRequest {
                    label: &self.label,
                    device_id: self.device_id,
                    shape,
                    options: run_options.as_ref(),
                },
                inputs,
                extract,
            )?;
            self.audit_placement_once(session)?;
            return Ok(result);
        }
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
        let outputs = match &run_options {
            Some(options) => session.run_binding_with_options(&binding, options),
            None => session.run_binding(&binding),
        }
        .map_err(|err| {
            config_invalid(format!(
                "ONNX io-binding inference failed for {}: {err}",
                self.label
            ))
        })?;
        let result = extract(&outputs)?;
        drop(outputs);
        drop(binding);
        self.audit_placement_once(session)?;
        Ok(result)
    }

    /// Once per session, after the first successful run, read the ORT profiling
    /// trace and audit per-provider node placement (#1142). No-op unless the
    /// operator enabled CALYX_ONNX_CPU_FALLBACK_AUDIT. Marked audited even on
    /// error so a failing gate does not re-run profiling every batch.
    fn audit_placement_once(&mut self, session: &mut Session) -> Result<()> {
        if !self.audit_mode.enabled() || self.audited {
            return Ok(());
        }
        self.audited = true;
        let trace_path = session.end_profiling().map_err(|err| {
            config_invalid(format!(
                "ONNX end_profiling failed for {}: {err}",
                self.label
            ))
        })?;
        let trace = std::fs::read_to_string(&trace_path).map_err(|err| {
            config_invalid(format!(
                "read ONNX profiling trace {trace_path} failed for {}: {err}",
                self.label
            ))
        })?;
        audit_from_trace(
            &self.label,
            &trace,
            self.gpu_policy,
            self.audit_mode,
            self.max_cpu_fraction,
        )
        .map(|_| ())
    }

    /// Records the run shape; returns whether it is first-seen. GPU sessions
    /// fail loud when distinct-shape diversity exceeds the configured cap —
    /// the ORT CUDA BFC arena retains per-shape allocations forever, so
    /// unbounded diversity is a slow-motion device OOM (#1143), and the
    /// batch/seq bucketing upstream keeps legitimate streams far below the
    /// cap.
    pub(super) fn enforce_shape_contract(&mut self, shape: (usize, usize)) -> Result<bool> {
        let new_shape = self.seen_shapes.insert(shape);
        if new_shape {
            eprintln!(
                "CALYX_ONNX_RUNTIME phase=io_binding_shape label={} batch={} seq={} io_binding={} distinct_shapes={}",
                self.label,
                shape.0,
                shape.1,
                self.io_binding,
                self.seen_shapes.len()
            );
            if self.gpu_policy && self.seen_shapes.len() > self.max_distinct_shapes {
                return Err(CalyxError {
                    code: "CALYX_ONNX_SHAPE_DIVERSITY",
                    message: format!(
                        "{} has run {} distinct (batch, seq) shapes, exceeding {MAX_DISTINCT_SHAPES_ENV}={} — unbounded shape diversity grows the CUDA BFC arena until device OOM (new shape batch={} seq={})",
                        self.label,
                        self.seen_shapes.len(),
                        self.max_distinct_shapes,
                        shape.0,
                        shape.1
                    ),
                    remediation: "batch and sequence bucketing should cap distinct shapes; find the caller that bypasses bucketed batching, or raise CALYX_ONNX_MAX_DISTINCT_SHAPES only if the workload legitimately needs more shape classes",
                });
            }
        }
        if !self.require_static {
            return Ok(new_shape);
        }
        match self.bound_shape {
            None => {
                self.bound_shape = Some(shape);
                Ok(new_shape)
            }
            Some(bound) if bound == shape => Ok(new_shape),
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
