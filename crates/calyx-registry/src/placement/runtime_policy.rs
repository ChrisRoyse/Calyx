use calyx_core::Placement;

use crate::{Bgem3Engine, LensRuntime};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum RuntimePlacementPolicy {
    CpuOnly,
    Dynamic,
    GpuOnly,
}

/// Selects the fail-closed placement used when hot-add has no measured budget.
///
/// Dynamic algorithmic runtimes remain CPU by default; budgeted admission may
/// promote them to GPU. Runtimes without a valid CPU path remain GPU-only.
pub fn runtime_default_placement(runtime: &LensRuntime) -> Placement {
    match runtime_placement_policy(runtime) {
        RuntimePlacementPolicy::CpuOnly | RuntimePlacementPolicy::Dynamic => Placement::Cpu,
        RuntimePlacementPolicy::GpuOnly => Placement::Gpu,
    }
}

pub(super) fn runtime_placement_policy(runtime: &LensRuntime) -> RuntimePlacementPolicy {
    match runtime {
        LensRuntime::Algorithmic { kind } if algorithmic_cuda_capable(kind) => {
            RuntimePlacementPolicy::Dynamic
        }
        LensRuntime::Algorithmic { .. }
        | LensRuntime::StaticLookup { .. }
        | LensRuntime::MultimodalAdapter { .. }
        | LensRuntime::ExternalCmd { .. }
        | LensRuntime::FastembedBgem3 {
            engine: Bgem3Engine::FastembedCpu,
            ..
        } => RuntimePlacementPolicy::CpuOnly,
        LensRuntime::TeiHttp { .. }
        | LensRuntime::TeiHttpSegmented { .. }
        | LensRuntime::CandleLocal { .. }
        | LensRuntime::Onnx { .. }
        | LensRuntime::OnnxColbert { .. }
        | LensRuntime::FastembedSparse { .. }
        | LensRuntime::FastembedBgem3 {
            engine: Bgem3Engine::OnnxCuda,
            ..
        }
        | LensRuntime::FastembedReranker { .. }
        | LensRuntime::FastembedQwen3 { .. } => RuntimePlacementPolicy::GpuOnly,
    }
}

fn algorithmic_cuda_capable(kind: &str) -> bool {
    matches!(
        kind,
        "byte"
            | "byte-features"
            | "byte_features"
            | "sparse"
            | "sparse-keywords"
            | "sparse_keywords"
            | "token-hash"
            | "token_hash"
            | "multi-hash"
            | "multi_hash"
    ) || kind.starts_with("sparse-keywords:")
        || kind.starts_with("sparse_keywords:")
        || kind.starts_with("token-hash:")
        || kind.starts_with("token_hash:")
}
