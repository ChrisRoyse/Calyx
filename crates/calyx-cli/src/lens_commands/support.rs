use std::sync::Arc;

use calyx_core::{CalyxError, Lens, LensId, Modality, Result, SlotShape, SlotVector};
use calyx_registry::{
    AlgorithmicLens, CandleLens, FastembedBgem3Lens, FastembedQwen3Lens, FastembedRerankerLens,
    FastembedSparseLens, FrozenLensContract, LensRuntime, LensSpec, MultimodalAdapterLens,
    NormPolicy, OnnxColbertLens, OnnxLens, Registry, StaticLookupLens, TeiHttpLens,
    TeiSegmentedLens, lens_spec_with_frozen_contract,
};

use crate::error::{CliError, CliResult};

pub(crate) struct PreparedRuntimeLens {
    pub(crate) lens: Arc<dyn Lens>,
    pub(crate) contract: FrozenLensContract,
    pub(crate) spec: LensSpec,
}

pub(crate) fn runtime_name(runtime: &LensRuntime) -> &'static str {
    match runtime {
        LensRuntime::Algorithmic { .. } => "algorithmic",
        LensRuntime::TeiHttp { .. } => "tei_http",
        LensRuntime::TeiHttpSegmented { .. } => "tei_http_segmented",
        LensRuntime::CandleLocal { .. } => "candle_local",
        LensRuntime::Onnx { .. } => "onnx",
        LensRuntime::OnnxColbert { .. } => "onnx_colbert",
        LensRuntime::FastembedSparse { .. } => "fastembed_sparse",
        LensRuntime::FastembedBgem3 { .. } => "fastembed_bgem3",
        LensRuntime::FastembedReranker { .. } => "fastembed_reranker",
        LensRuntime::FastembedQwen3 { .. } => "fastembed_qwen3",
        LensRuntime::StaticLookup { .. } => "static_lookup",
        LensRuntime::MultimodalAdapter { .. } => "multimodal_adapter",
        LensRuntime::ExternalCmd { .. } => "external_cmd",
    }
}

pub(crate) fn register_manifest_runtime(registry: &mut Registry, spec: LensSpec) -> Result<LensId> {
    register_prepared_manifest_runtime(registry, prepare_manifest_runtime(spec)?)
}

pub(crate) fn register_prepared_manifest_runtime(
    registry: &mut Registry,
    prepared: PreparedRuntimeLens,
) -> Result<LensId> {
    registry.register_frozen_arc_with_spec(prepared.lens, prepared.contract, prepared.spec)
}

pub(crate) fn prepare_manifest_runtime(spec: LensSpec) -> Result<PreparedRuntimeLens> {
    match &spec.runtime {
        LensRuntime::Onnx { .. } => {
            let lens = OnnxLens::from_lens_spec(&spec)?;
            let contract = lens.contract().clone();
            Ok(prepared(lens, contract, spec))
        }
        LensRuntime::OnnxColbert { .. } => {
            let lens = OnnxColbertLens::from_lens_spec(&spec)?;
            let contract = lens.contract().clone();
            Ok(prepared(lens, contract, spec))
        }
        LensRuntime::FastembedSparse { .. } => {
            let lens = FastembedSparseLens::from_lens_spec(&spec)?;
            let contract = lens.contract().clone();
            Ok(prepared(lens, contract, spec))
        }
        LensRuntime::FastembedBgem3 { .. } => {
            let lens = FastembedBgem3Lens::from_lens_spec(&spec)?;
            let contract = lens.contract().clone();
            Ok(prepared(lens, contract, spec))
        }
        LensRuntime::FastembedReranker { .. } => {
            let lens = FastembedRerankerLens::from_lens_spec(&spec)?;
            let contract = lens.contract().clone();
            Ok(prepared(lens, contract, spec))
        }
        LensRuntime::FastembedQwen3 { .. } => {
            let lens = FastembedQwen3Lens::from_lens_spec(&spec)?;
            let contract = lens.contract().clone();
            Ok(prepared(lens, contract, spec))
        }
        LensRuntime::CandleLocal { .. } => {
            let lens = CandleLens::from_lens_spec(&spec)?;
            let contract = lens.contract().clone();
            Ok(prepared(lens, contract, spec))
        }
        LensRuntime::StaticLookup { .. } => {
            let lens = StaticLookupLens::from_lens_spec(&spec)?;
            let contract = lens.contract().clone();
            Ok(prepared(lens, contract, spec))
        }
        LensRuntime::MultimodalAdapter { .. } => {
            let lens = MultimodalAdapterLens::from_lens_spec(&spec)?;
            let contract = lens.contract();
            Ok(prepared(lens, contract, spec))
        }
        LensRuntime::TeiHttp { endpoint } => {
            let lens = TeiHttpLens::new(&spec.name, endpoint, spec.modality, dim(spec.output))
                .with_declared_max_batch(spec.max_batch)?;
            let contract =
                FrozenLensContract::tei_http(&spec.name, endpoint, spec.modality, dim(spec.output));
            Ok(prepared(lens, contract, spec))
        }
        LensRuntime::TeiHttpSegmented { endpoint, policy } => {
            let lens = TeiSegmentedLens::new(
                &spec.name,
                endpoint,
                spec.modality,
                dim(spec.output),
                *policy,
            )?
            .with_declared_max_batch(spec.max_batch)?;
            let contract = lens.contract(&spec.name);
            Ok(prepared(lens, contract, spec))
        }
        LensRuntime::Algorithmic { kind } => {
            let lens = algorithmic_lens(&spec.name, spec.modality, kind, spec.output)?;
            let contract = lens.contract().clone();
            Ok(prepared(lens, contract, spec))
        }
        LensRuntime::ExternalCmd { .. } => Err(CalyxError::lens_unreachable(
            "manifest runtime registration does not load external-cmd lenses",
        )),
    }
}

fn prepared<L>(lens: L, contract: FrozenLensContract, spec: LensSpec) -> PreparedRuntimeLens
where
    L: Lens + 'static,
{
    let spec = lens_spec_with_frozen_contract(spec, &contract);
    PreparedRuntimeLens {
        lens: Arc::new(lens),
        contract,
        spec,
    }
}

pub(crate) fn dim(shape: SlotShape) -> u32 {
    match shape {
        SlotShape::Dense(dim) | SlotShape::Sparse(dim) => dim,
        SlotShape::Multi { token_dim } => token_dim,
    }
}

fn algorithmic_lens(
    name: &str,
    modality: Modality,
    kind: &str,
    shape: SlotShape,
) -> Result<AlgorithmicLens> {
    AlgorithmicLens::from_runtime_kind(name, modality, kind, shape)
}

pub(crate) fn hex_from_bytes(bytes: &[u8; 32]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub(crate) fn slot_norm(vector: &SlotVector) -> f32 {
    match vector {
        SlotVector::Dense { data, .. } => {
            data.iter().map(|value| value * value).sum::<f32>().sqrt()
        }
        SlotVector::Sparse { entries, .. } => entries
            .iter()
            .map(|entry| entry.val * entry.val)
            .sum::<f32>()
            .sqrt(),
        SlotVector::Multi { tokens, .. } => tokens
            .iter()
            .flat_map(|token| token.iter())
            .map(|value| value * value)
            .sum::<f32>()
            .sqrt(),
        SlotVector::SegmentedMulti { segments, .. } => segments
            .iter()
            .flat_map(|segment| segment.data.iter())
            .map(|value| value * value)
            .sum::<f32>()
            .sqrt(),
        SlotVector::Absent { .. } => 0.0,
    }
}

pub(crate) fn slot_prefix(vector: &SlotVector, limit: usize) -> Vec<f32> {
    match vector {
        SlotVector::Dense { data, .. } => data.iter().take(limit).copied().collect(),
        SlotVector::Sparse { dim, entries } => {
            let mut values = vec![0.0; (*dim as usize).min(limit)];
            for entry in entries {
                if let Some(value) = values.get_mut(entry.idx as usize) {
                    *value = entry.val;
                }
            }
            values
        }
        SlotVector::Multi { tokens, .. } => tokens
            .first()
            .map(|token| token.iter().take(limit).copied().collect())
            .unwrap_or_default(),
        SlotVector::SegmentedMulti { segments, .. } => segments
            .first()
            .map(|segment| segment.data.iter().take(limit).copied().collect())
            .unwrap_or_default(),
        SlotVector::Absent { .. } => Vec::new(),
    }
}

pub(crate) fn validate_vector_contract(
    vector: &SlotVector,
    expected_shape: SlotShape,
    norm_policy: NormPolicy,
) -> CliResult<()> {
    validate_vector_shape(vector, expected_shape)?;
    if has_non_finite(vector) {
        return Err(CliError::from(CalyxError::lens_numerical_invariant(
            "lens explain received NaN or Inf from runtime",
        )));
    }
    if let SlotVector::SegmentedMulti { segments, .. } = vector {
        for (index, segment) in segments.iter().enumerate() {
            let norm = segment
                .data
                .iter()
                .map(|value| value * value)
                .sum::<f32>()
                .sqrt();
            match norm_policy {
                NormPolicy::L2 { tolerance } | NormPolicy::Unit { tolerance } => {
                    ensure_norm_close(norm, 1.0, tolerance).map_err(|error| {
                        CliError::from(CalyxError::lens_numerical_invariant(format!(
                            "segmented lens vector {index} violates its unit norm: {error}"
                        )))
                    })?;
                }
                NormPolicy::DeclaredByModel {
                    declared_norm,
                    tolerance,
                } => ensure_norm_close(norm, declared_norm, tolerance).map_err(|error| {
                    CliError::from(CalyxError::lens_numerical_invariant(format!(
                        "segmented lens vector {index} violates declared norm: {error}"
                    )))
                })?,
                NormPolicy::None | NormPolicy::Finite => {}
            }
        }
        return Ok(());
    }
    let norm = slot_norm(vector);
    match norm_policy {
        NormPolicy::L2 { tolerance } | NormPolicy::Unit { tolerance } => {
            ensure_norm_close(norm, 1.0, tolerance)
        }
        NormPolicy::DeclaredByModel {
            declared_norm,
            tolerance,
        } => ensure_norm_close(norm, declared_norm, tolerance),
        NormPolicy::None | NormPolicy::Finite => Ok(()),
    }
}

fn validate_vector_shape(vector: &SlotVector, expected_shape: SlotShape) -> CliResult {
    match (vector, expected_shape) {
        (SlotVector::Dense { dim, data }, SlotShape::Dense(expected)) => {
            if *dim == expected && data.len() == expected as usize {
                return Ok(());
            }
            Err(CalyxError::lens_dim_mismatch(format!(
                "lens explain dense dim {dim} len {} != declared {expected}",
                data.len()
            )))?
        }
        (SlotVector::Sparse { dim, .. }, SlotShape::Sparse(expected)) if *dim == expected => Ok(()),
        (
            SlotVector::Multi { token_dim, .. },
            SlotShape::Multi {
                token_dim: expected,
            },
        ) if *token_dim == expected => Ok(()),
        (
            SlotVector::SegmentedMulti { token_dim, .. },
            SlotShape::Multi {
                token_dim: expected,
            },
        ) if *token_dim == expected => Ok(()),
        (SlotVector::Absent { .. }, _) => Err(CalyxError::lens_dim_mismatch(
            "lens explain runtime returned an absent vector",
        ))?,
        (_, expected) => Err(CalyxError::lens_dim_mismatch(format!(
            "lens explain vector shape does not match declared {expected:?}"
        )))?,
    }
}

fn has_non_finite(vector: &SlotVector) -> bool {
    match vector {
        SlotVector::Dense { data, .. } => data.iter().any(|value| !value.is_finite()),
        SlotVector::Sparse { entries, .. } => entries.iter().any(|entry| !entry.val.is_finite()),
        SlotVector::Multi { tokens, .. } => tokens
            .iter()
            .flat_map(|token| token.iter())
            .any(|value| !value.is_finite()),
        SlotVector::SegmentedMulti { segments, .. } => segments
            .iter()
            .flat_map(|segment| segment.data.iter())
            .any(|value| !value.is_finite()),
        SlotVector::Absent { .. } => false,
    }
}

fn ensure_norm_close(actual: f32, expected: f32, tolerance: f32) -> CliResult {
    if actual.is_finite()
        && expected.is_finite()
        && tolerance.is_finite()
        && (actual - expected).abs() <= tolerance
    {
        return Ok(());
    }
    Err(CliError::from(CalyxError::lens_numerical_invariant(
        format!("lens explain norm {actual} outside expected {expected} +/- {tolerance}"),
    )))
}
