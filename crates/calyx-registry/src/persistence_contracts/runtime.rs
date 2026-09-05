use std::sync::Arc;

use calyx_core::{CalyxError, Lens, Result, SlotShape};

use crate::frozen::{FrozenLensContract, LensDType, NormPolicy, sha256_digest};
use crate::relocation::{self, EndpointRelocation};
use crate::{
    AlgorithmicLens, CandleLens, ExternalCmdLens, FastembedBgem3Lens, FastembedQwen3Lens,
    FastembedRerankerLens, FastembedSparseLens, LensRuntime, LensSpec, MultimodalAdapterLens,
    OnnxColbertLens, OnnxLens, StaticLookupLens, TeiHttpLens, TeiSegmentedLens,
};

/// Resolves an audited relocation for a frozen TEI endpoint and proves, before
/// the lens is admitted, that the relocated lane is the exact instrument the
/// relocation record declares.
///
/// The behavioural proof is the persisted runtime golden replayed by
/// [`crate::persistence`]; this check exists so that a mis-pointed relocation
/// reports the precise mismatched `/info` field instead of surfacing later as
/// an opaque `CALYX_LENS_RUNTIME_DRIFT`.
fn relocation_for(frozen_endpoint: &str) -> Result<Option<&'static EndpointRelocation>> {
    let Some(record) = relocation::resolve(frozen_endpoint)? else {
        return Ok(None);
    };
    relocation::verify_relocated_service(record)?;
    Ok(Some(record))
}

pub(crate) fn load_runtime_lens_from_spec(
    spec: &LensSpec,
) -> Result<(Arc<dyn Lens>, FrozenLensContract)> {
    match &spec.runtime {
        LensRuntime::Algorithmic { kind } => {
            let lens =
                AlgorithmicLens::from_runtime_kind(&spec.name, spec.modality, kind, spec.output)?;
            let contract = lens.contract().clone();
            Ok((Arc::new(lens), contract))
        }
        LensRuntime::TeiHttp { endpoint } => {
            let dim = dense_dim(spec.output).ok_or_else(|| {
                lens_config_invalid(format!(
                    "TEI lens {} requires dense output shape, got {:?}",
                    spec.name, spec.output
                ))
            })?;
            let lens = TeiHttpLens::new(&spec.name, endpoint, spec.modality, dim)
                .with_declared_max_batch(spec.max_batch)?;
            let lens = match relocation_for(endpoint)? {
                Some(record) => lens.relocated_to(&record.live_endpoint)?,
                None => lens,
            };
            // Derived from the frozen endpoint in the spec, never from the
            // relocated one, so a rebinding cannot change the LensId (#2053).
            let contract = FrozenLensContract::tei_http(&spec.name, endpoint, spec.modality, dim);
            Ok((Arc::new(lens), contract))
        }
        LensRuntime::TeiHttpSegmented { endpoint, policy } => {
            // Pooled lanes emit one dense vector per document (#1980); only
            // segment-preserving lanes carry a multi shape.
            let dim = if policy.is_pooled() {
                dense_dim(spec.output)
            } else {
                multi_dim(spec.output)
            }
            .ok_or_else(|| {
                let expected = if policy.is_pooled() { "dense" } else { "multi" };
                lens_config_invalid(format!(
                    "segmented TEI lens {} requires {expected} output shape, got {:?}",
                    spec.name, spec.output
                ))
            })?;
            let lens = TeiSegmentedLens::new(&spec.name, endpoint, spec.modality, dim, *policy)?
                .with_declared_max_batch(spec.max_batch)?;
            let lens = match relocation_for(endpoint)? {
                Some(record) => lens.relocated_to(&record.live_endpoint)?,
                None => lens,
            };
            // `contract` reads the lens's frozen endpoint, so this stays the
            // commissioned identity even when the lens dials a relocated lane.
            let contract = lens.contract(&spec.name);
            Ok((Arc::new(lens), contract))
        }
        LensRuntime::ExternalCmd { cmd, args } => {
            let dim = dense_dim(spec.output).ok_or_else(|| {
                lens_config_invalid(format!(
                    "external command lens {} requires dense output shape, got {:?}",
                    spec.name, spec.output
                ))
            })?;
            let lens = ExternalCmdLens::new(&spec.name, cmd, args.clone(), spec.modality, dim);
            let args_text = args.join("\0");
            let contract = FrozenLensContract::new(
                spec.name.clone(),
                sha256_digest(&[cmd.as_bytes(), args_text.as_bytes()]),
                sha256_digest(&[b"external-cmd-runtime-v1"]),
                SlotShape::Dense(dim),
                spec.modality,
                LensDType::F32,
                NormPolicy::None,
            );
            Ok((Arc::new(lens), contract))
        }
        LensRuntime::CandleLocal { .. } => {
            let lens = CandleLens::from_lens_spec(spec)?;
            let contract = lens.contract().clone();
            Ok((Arc::new(lens), contract))
        }
        LensRuntime::Onnx { .. } => {
            let lens = OnnxLens::from_lens_spec(spec)?;
            let contract = lens.contract().clone();
            Ok((Arc::new(lens), contract))
        }
        LensRuntime::OnnxColbert { .. } => {
            let lens = OnnxColbertLens::from_lens_spec(spec)?;
            let contract = lens.contract().clone();
            Ok((Arc::new(lens), contract))
        }
        LensRuntime::FastembedSparse { .. } => {
            let lens = FastembedSparseLens::from_lens_spec(spec)?;
            let contract = lens.contract().clone();
            Ok((Arc::new(lens), contract))
        }
        LensRuntime::FastembedBgem3 { .. } => {
            let lens = FastembedBgem3Lens::from_lens_spec(spec)?;
            let contract = lens.contract().clone();
            Ok((Arc::new(lens), contract))
        }
        LensRuntime::FastembedReranker { .. } => {
            let lens = FastembedRerankerLens::from_lens_spec(spec)?;
            let contract = lens.contract().clone();
            Ok((Arc::new(lens), contract))
        }
        LensRuntime::FastembedQwen3 { .. } => {
            let lens = FastembedQwen3Lens::from_lens_spec(spec)?;
            let contract = lens.contract().clone();
            Ok((Arc::new(lens), contract))
        }
        LensRuntime::StaticLookup { .. } => {
            let lens = StaticLookupLens::from_lens_spec(spec)?;
            let contract = lens.contract().clone();
            Ok((Arc::new(lens), contract))
        }
        LensRuntime::MultimodalAdapter { .. } => {
            let lens = MultimodalAdapterLens::from_lens_spec(spec)?;
            let contract = lens.contract();
            Ok((Arc::new(lens), contract))
        }
    }
}

pub fn algorithmic_lens_from_spec(spec: &LensSpec) -> Result<AlgorithmicLens> {
    let LensRuntime::Algorithmic { kind } = &spec.runtime else {
        return Err(lens_config_invalid(format!(
            "lens {} is not an algorithmic runtime",
            spec.name
        )));
    };
    AlgorithmicLens::from_runtime_kind(&spec.name, spec.modality, kind, spec.output)
}

fn dense_dim(shape: SlotShape) -> Option<u32> {
    match shape {
        SlotShape::Dense(dim) => Some(dim),
        SlotShape::Sparse(_) | SlotShape::Multi { .. } => None,
    }
}

fn multi_dim(shape: SlotShape) -> Option<u32> {
    match shape {
        SlotShape::Multi { token_dim } => Some(token_dim),
        SlotShape::Dense(_) | SlotShape::Sparse(_) => None,
    }
}

fn lens_config_invalid(message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: "CALYX_LENS_CONFIG_INVALID",
        message: message.into(),
        remediation: "fix persisted LensSpec runtime fields or re-register the lens",
    }
}
