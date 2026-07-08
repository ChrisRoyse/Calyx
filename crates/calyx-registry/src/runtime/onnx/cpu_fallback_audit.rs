//! Per-provider node-placement audit for GPU-policy ONNX sessions (#1142).
//!
//! The CUDA execution provider has no kernels for int8-quantized ops
//! (`QLinearMatMul`, `QGemm`, `MatMulInteger`, `DynamicQuantizeLinear`,
//! `ConvInteger`, …). ORT silently places those nodes on the implicit CPU EP,
//! so a session that reports `provider=CudaFailLoud` can execute most of its
//! compute on the CPU with a device↔host copy per node — measured at
//! 130–250 ms/input, unusable for bulk encode. The `session_ready` telemetry
//! said "gpu" while execution was CPU-bound (#1142), and `#1136`'s I/O binding
//! cannot fix it — it addresses the copy path of GPU-executable graphs.
//!
//! This audit parses the ORT profiling trace after the first real run, counts
//! compute nodes per execution provider, emits the per-provider counts in the
//! readback telemetry (so the fallback is *visible*, not inferred), and — in
//! `fail` mode — refuses a GPU-policy session that runs more than a configured
//! fraction of its compute nodes on CPU. The pure parsing and policy functions
//! are exercised directly by unit tests with synthetic traces; the GPU run that
//! populates a real trace is exercised by the lens runtimes on device.
//!
//! Environment knobs:
//! - `CALYX_ONNX_CPU_FALLBACK_AUDIT` — `off` (default) | `warn` | `fail`. When
//!   not `off`, ORT profiling is enabled at session build and the audit runs
//!   once after the first successful inference. `warn` logs the per-provider
//!   counts; `fail` additionally errors when a GPU-policy session is over the
//!   CPU-node fraction. Default `off` keeps the hot path unchanged; CudaFailLoud
//!   is still protected at build time by `session.disable_cpu_ep_fallback=1`.
//! - `CALYX_ONNX_MAX_CPU_NODE_FRACTION` — CPU compute-node fraction a GPU-policy
//!   session may reach before `fail` refuses it (default 0.10, range [0,1]).

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use calyx_core::{CalyxError, Result};
use serde_json::Value;

pub(super) const CPU_FALLBACK_AUDIT_ENV: &str = "CALYX_ONNX_CPU_FALLBACK_AUDIT";
pub(super) const MAX_CPU_NODE_FRACTION_ENV: &str = "CALYX_ONNX_MAX_CPU_NODE_FRACTION";
pub(super) const CPU_FALLBACK_CODE: &str = "CALYX_ONNX_QUANT_CPU_FALLBACK";

const DEFAULT_MAX_CPU_FRACTION: f64 = 0.10;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum AuditMode {
    Off,
    Warn,
    Fail,
}

impl AuditMode {
    pub(super) const fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Warn => "warn",
            Self::Fail => "fail",
        }
    }

    pub(super) const fn enabled(self) -> bool {
        !matches!(self, Self::Off)
    }
}

pub(super) fn configured_audit_mode() -> Result<AuditMode> {
    let Ok(raw) = std::env::var(CPU_FALLBACK_AUDIT_ENV) else {
        return Ok(AuditMode::Off);
    };
    match raw.trim().to_ascii_lowercase().as_str() {
        "" | "off" | "0" | "false" => Ok(AuditMode::Off),
        "warn" => Ok(AuditMode::Warn),
        "fail" | "1" | "true" => Ok(AuditMode::Fail),
        other => Err(CalyxError {
            code: "CALYX_ONNX_CPU_FALLBACK_AUDIT_INVALID",
            message: format!("{CPU_FALLBACK_AUDIT_ENV}={other} is not off, warn, or fail"),
            remediation: "set CALYX_ONNX_CPU_FALLBACK_AUDIT to off, warn, or fail (default off)",
        }),
    }
}

pub(super) fn configured_max_cpu_fraction() -> Result<f64> {
    let Ok(raw) = std::env::var(MAX_CPU_NODE_FRACTION_ENV) else {
        return Ok(DEFAULT_MAX_CPU_FRACTION);
    };
    let raw = raw.trim();
    if raw.is_empty() {
        return Ok(DEFAULT_MAX_CPU_FRACTION);
    }
    raw.parse::<f64>()
        .ok()
        .filter(|fraction| fraction.is_finite() && (0.0..=1.0).contains(fraction))
        .ok_or_else(|| CalyxError {
            code: "CALYX_ONNX_MAX_CPU_NODE_FRACTION_INVALID",
            message: format!("{MAX_CPU_NODE_FRACTION_ENV}={raw} is not a fraction in [0, 1]"),
            remediation: "set CALYX_ONNX_MAX_CPU_NODE_FRACTION to a value in [0, 1] (default 0.10), or unset it",
        })
}

/// A unique, writable profiling trace path for a session. ORT appends its own
/// timestamp and `.json` suffix and returns the final path from `end_profiling`.
pub(super) fn profiling_file_path(label: &str) -> PathBuf {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    let slug: String = label
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '_' })
        .collect();
    std::env::temp_dir().join(format!(
        "calyx_onnx_profile_{}_{}_{seq}",
        std::process::id(),
        slug
    ))
}

/// Compute-node counts keyed by ORT execution-provider name.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct ProviderNodeCounts {
    counts: BTreeMap<String, usize>,
}

impl ProviderNodeCounts {
    fn add(&mut self, provider: &str) {
        *self.counts.entry(provider.to_string()).or_default() += 1;
    }

    fn total(&self) -> usize {
        self.counts.values().copied().sum()
    }

    fn cpu_nodes(&self) -> usize {
        self.counts
            .iter()
            .filter(|(provider, _)| is_cpu_provider(provider))
            .map(|(_, count)| *count)
            .sum()
    }

    fn render(&self) -> String {
        if self.counts.is_empty() {
            return "none".to_string();
        }
        self.counts
            .iter()
            .map(|(provider, count)| format!("{provider}:{count}"))
            .collect::<Vec<_>>()
            .join(",")
    }
}

fn is_cpu_provider(provider: &str) -> bool {
    provider.to_ascii_uppercase().contains("CPU")
}

/// Parse an ORT profiling trace into per-provider compute-node counts.
///
/// ORT emits three events per node (`_fence_before`, `_kernel_time`,
/// `_fence_after`); the `_kernel_time` record is the actual compute and carries
/// `args.provider`. We count those. Some ORT builds omit the suffix, so if no
/// `_kernel_time` records are present we fall back to every `cat=="Node"` event
/// carrying a provider.
pub(super) fn parse_profiling_nodes(trace_json: &str) -> Result<ProviderNodeCounts> {
    let value: Value = serde_json::from_str(trace_json).map_err(|err| CalyxError {
        code: "CALYX_ONNX_PROFILE_PARSE",
        message: format!("ONNX profiling trace is not valid JSON: {err}"),
        remediation: "the ORT profiling trace is malformed; rerun with CALYX_ONNX_CPU_FALLBACK_AUDIT and check ORT version",
    })?;
    let events = match &value {
        Value::Array(events) => events.as_slice(),
        Value::Object(map) => match map.get("traceEvents") {
            Some(Value::Array(events)) => events.as_slice(),
            _ => {
                return Err(CalyxError {
                    code: "CALYX_ONNX_PROFILE_PARSE",
                    message: "ONNX profiling trace object has no traceEvents array".to_string(),
                    remediation: "expected an ORT profiling trace (JSON array or {traceEvents:[...]})",
                });
            }
        },
        _ => {
            return Err(CalyxError {
                code: "CALYX_ONNX_PROFILE_PARSE",
                message: "ONNX profiling trace is neither an array nor a traceEvents object"
                    .to_string(),
                remediation: "expected an ORT profiling trace (JSON array or {traceEvents:[...]})",
            });
        }
    };

    let mut kernel = ProviderNodeCounts::default();
    let mut any_node = ProviderNodeCounts::default();
    for event in events {
        let Some(obj) = event.as_object() else {
            continue;
        };
        if obj.get("cat").and_then(Value::as_str) != Some("Node") {
            continue;
        }
        let Some(provider) = obj
            .get("args")
            .and_then(Value::as_object)
            .and_then(|args| args.get("provider"))
            .and_then(Value::as_str)
            .filter(|provider| !provider.trim().is_empty())
        else {
            continue;
        };
        any_node.add(provider);
        if obj
            .get("name")
            .and_then(Value::as_str)
            .is_some_and(|name| name.ends_with("_kernel_time"))
        {
            kernel.add(provider);
        }
    }
    Ok(if kernel.total() > 0 { kernel } else { any_node })
}

/// The verdict of a placement audit — the numbers that also go to telemetry.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct CpuFallbackAudit {
    pub(super) total_nodes: usize,
    pub(super) cpu_nodes: usize,
    pub(super) cpu_fraction: f64,
    pub(super) max_cpu_fraction: f64,
    pub(super) over_threshold: bool,
    pub(super) per_provider: String,
}

pub(super) fn evaluate_placement(
    counts: &ProviderNodeCounts,
    gpu_policy: bool,
    max_cpu_fraction: f64,
) -> CpuFallbackAudit {
    let total_nodes = counts.total();
    let cpu_nodes = counts.cpu_nodes();
    let cpu_fraction = if total_nodes == 0 {
        0.0
    } else {
        cpu_nodes as f64 / total_nodes as f64
    };
    // Strictly greater than the budget so an exact-threshold panel passes, and
    // a session with no measured nodes never trips (nothing to judge).
    let over_threshold = gpu_policy && total_nodes > 0 && cpu_fraction > max_cpu_fraction;
    CpuFallbackAudit {
        total_nodes,
        cpu_nodes,
        cpu_fraction,
        max_cpu_fraction,
        over_threshold,
        per_provider: counts.render(),
    }
}

/// Parse the trace, evaluate placement, emit telemetry, and — in `fail` mode —
/// refuse a GPU-policy session that is over the CPU-node fraction.
pub(super) fn audit_from_trace(
    label: &str,
    trace_json: &str,
    gpu_policy: bool,
    mode: AuditMode,
    max_cpu_fraction: f64,
) -> Result<CpuFallbackAudit> {
    let counts = parse_profiling_nodes(trace_json)?;
    let audit = evaluate_placement(&counts, gpu_policy, max_cpu_fraction);
    let verdict = if audit.over_threshold { "over" } else { "ok" };
    eprintln!(
        "CALYX_ONNX_RUNTIME phase=cpu_fallback_audit label={label} mode={} gpu_policy={gpu_policy} total_nodes={} cpu_nodes={} cpu_fraction={:.4} max_cpu_fraction={:.4} providers={} verdict={verdict}",
        mode.as_str(),
        audit.total_nodes,
        audit.cpu_nodes,
        audit.cpu_fraction,
        audit.max_cpu_fraction,
        audit.per_provider,
    );
    if audit.over_threshold && mode == AuditMode::Fail {
        return Err(CalyxError {
            code: CPU_FALLBACK_CODE,
            message: format!(
                "{label} claims a GPU execution provider but ran {}/{} compute nodes ({:.1}%) on CPU (providers={}), exceeding {MAX_CPU_NODE_FRACTION_ENV}={:.4} — int8/quantized ONNX graphs have no CUDA kernels, so QLinearMatMul/QGemm/MatMulInteger fall back to CPU per node with a device<->host copy each way",
                audit.cpu_nodes,
                audit.total_nodes,
                audit.cpu_fraction * 100.0,
                audit.per_provider,
                audit.max_cpu_fraction,
            ),
            remediation: "prefer the fp16/fp32 ONNX variant of this lens for bulk encode (assay corpus-build / stream-fbin / ingest); keep int8 graphs for resident low-VRAM serving under CPU policy, or raise CALYX_ONNX_MAX_CPU_NODE_FRACTION only if this mixed placement is expected",
        });
    }
    Ok(audit)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kernel_event(name: &str, provider: &str) -> Value {
        serde_json::json!({
            "cat": "Node",
            "name": name,
            "dur": 42,
            "ph": "X",
            "args": {"op_name": "MatMul", "provider": provider}
        })
    }

    #[test]
    fn parses_kernel_time_nodes_per_provider() {
        let trace = serde_json::json!([
            {"cat": "Session", "name": "model_loading", "args": {}},
            kernel_event("MatMul_kernel_time", "CUDAExecutionProvider"),
            kernel_event("Add_kernel_time", "CUDAExecutionProvider"),
            kernel_event("QLinearMatMul_kernel_time", "CPUExecutionProvider"),
            // fence events must not be counted as separate compute nodes.
            {"cat": "Node", "name": "MatMul_fence_before", "args": {"provider": "CUDAExecutionProvider"}},
            {"cat": "Node", "name": "MatMul_fence_after", "args": {"provider": "CUDAExecutionProvider"}},
        ])
        .to_string();
        let counts = parse_profiling_nodes(&trace).unwrap();
        assert_eq!(counts.total(), 3);
        assert_eq!(counts.cpu_nodes(), 1);
    }

    #[test]
    fn parses_trace_events_wrapper_and_falls_back_without_kernel_suffix() {
        let trace = serde_json::json!({
            "traceEvents": [
                {"cat": "Node", "name": "MatMul", "args": {"provider": "CUDAExecutionProvider"}},
                {"cat": "Node", "name": "QGemm", "args": {"provider": "CPUExecutionProvider"}},
            ]
        })
        .to_string();
        let counts = parse_profiling_nodes(&trace).unwrap();
        assert_eq!(counts.total(), 2);
        assert_eq!(counts.cpu_nodes(), 1);
    }

    #[test]
    fn malformed_trace_fails_closed() {
        assert_eq!(
            parse_profiling_nodes("not json").unwrap_err().code,
            "CALYX_ONNX_PROFILE_PARSE"
        );
        assert_eq!(
            parse_profiling_nodes("{\"nope\": 1}").unwrap_err().code,
            "CALYX_ONNX_PROFILE_PARSE"
        );
    }

    #[test]
    fn gpu_policy_over_threshold_fails_loud() {
        // A predominantly-int8 graph: 20 CPU nodes, 4 CUDA nodes → 83% CPU.
        let mut counts = ProviderNodeCounts::default();
        for _ in 0..20 {
            counts.add("CPUExecutionProvider");
        }
        for _ in 0..4 {
            counts.add("CUDAExecutionProvider");
        }
        let audit = evaluate_placement(&counts, true, 0.10);
        assert_eq!(audit.total_nodes, 24);
        assert_eq!(audit.cpu_nodes, 20);
        assert!(audit.over_threshold);

        let err = audit_from_trace_counts(&counts, true, AuditMode::Fail, 0.10).unwrap_err();
        assert_eq!(err.code, CPU_FALLBACK_CODE);
        assert!(err.message.contains("83"), "{}", err.message);
    }

    #[test]
    fn warn_mode_never_errors_even_over_threshold() {
        let mut counts = ProviderNodeCounts::default();
        for _ in 0..10 {
            counts.add("CPUExecutionProvider");
        }
        let audit = audit_from_trace_counts(&counts, true, AuditMode::Warn, 0.10).unwrap();
        assert!(audit.over_threshold);
    }

    #[test]
    fn gpu_session_fully_on_device_passes() {
        let mut counts = ProviderNodeCounts::default();
        for _ in 0..120 {
            counts.add("CUDAExecutionProvider");
        }
        // Two legit CPU nodes (e.g. a Cast) stay under the 10% budget.
        counts.add("CPUExecutionProvider");
        counts.add("CPUExecutionProvider");
        let audit = audit_from_trace_counts(&counts, true, AuditMode::Fail, 0.10).unwrap();
        assert!(!audit.over_threshold);
        assert!(audit.cpu_fraction < 0.10);
    }

    #[test]
    fn cpu_policy_is_never_flagged() {
        let mut counts = ProviderNodeCounts::default();
        for _ in 0..50 {
            counts.add("CPUExecutionProvider");
        }
        // gpu_policy=false: an all-CPU session under CPU policy is correct.
        let audit = audit_from_trace_counts(&counts, false, AuditMode::Fail, 0.10).unwrap();
        assert!(!audit.over_threshold);
    }

    #[test]
    fn mode_and_fraction_env_parsing() {
        assert!(configured_max_cpu_fraction().unwrap() > 0.0);
        assert!(!AuditMode::Off.enabled());
        assert!(AuditMode::Fail.enabled());
    }

    // Test shim mirroring `audit_from_trace` but taking counts directly, so the
    // policy path is exercised without synthesizing a full JSON trace.
    fn audit_from_trace_counts(
        counts: &ProviderNodeCounts,
        gpu_policy: bool,
        mode: AuditMode,
        max_cpu_fraction: f64,
    ) -> Result<CpuFallbackAudit> {
        let audit = evaluate_placement(counts, gpu_policy, max_cpu_fraction);
        if audit.over_threshold && mode == AuditMode::Fail {
            return Err(CalyxError {
                code: CPU_FALLBACK_CODE,
                message: format!(
                    "ran {}/{} compute nodes ({:.1}%) on CPU",
                    audit.cpu_nodes,
                    audit.total_nodes,
                    audit.cpu_fraction * 100.0
                ),
                remediation: "prefer the fp variant",
            });
        }
        Ok(audit)
    }
}
