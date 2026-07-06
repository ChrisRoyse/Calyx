use std::sync::Arc;

use calyx_core::{CalyxError, Lens, Result, SlotShape};

use crate::frozen::{FrozenLensContract, LensDType, NormPolicy, sha256_digest};
use crate::{
    AlgorithmicLens, CandleLens, ExternalCmdLens, FastembedBgem3Lens, FastembedQwen3Lens,
    FastembedRerankerLens, FastembedSparseLens, LensRuntime, LensSpec, MultimodalAdapterLens,
    OnnxColbertLens, OnnxLens, StaticLookupLens, TeiHttpLens,
};

pub(crate) fn load_runtime_lens_from_spec(
    spec: &LensSpec,
) -> Result<(Arc<dyn Lens>, FrozenLensContract)> {
    match &spec.runtime {
        LensRuntime::Algorithmic { kind } => {
            let lens = algorithmic_lens(spec, kind).ok_or_else(|| {
                lens_config_invalid(format!(
                    "unsupported algorithmic lens kind {kind} for persisted lens {}",
                    spec.name
                ))
            })?;
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
            let lens = TeiHttpLens::new(&spec.name, endpoint, spec.modality, dim);
            let contract = FrozenLensContract::tei_http(&spec.name, endpoint, spec.modality, dim);
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

fn algorithmic_lens(spec: &LensSpec, kind: &str) -> Option<AlgorithmicLens> {
    match kind {
        "byte_features" | "byte-features" | "byte" => {
            Some(AlgorithmicLens::byte_features(&spec.name, spec.modality))
        }
        "scalar" => Some(AlgorithmicLens::scalar(&spec.name, spec.modality)),
        "ast_style" | "ast-style" => Some(AlgorithmicLens::ast_style(&spec.name, spec.modality)),
        "gdelt_cameo" | "gdelt-cameo" => {
            Some(AlgorithmicLens::gdelt_cameo(&spec.name, spec.modality))
        }
        "gdelt_actor_geo" | "gdelt-actor-geo" => Some(AlgorithmicLens::gdelt_actor_geo(
            &spec.name,
            spec.modality,
            sparse_dim(spec.output)?,
        )),
        "gdelt_source_domain" | "gdelt-source-domain" => {
            Some(AlgorithmicLens::gdelt_source_domain(
                &spec.name,
                spec.modality,
                sparse_dim(spec.output)?,
            ))
        }
        "gdelt_event_geo" | "gdelt-event-geo" => Some(AlgorithmicLens::gdelt_event_geo(
            &spec.name,
            spec.modality,
            sparse_dim(spec.output)?,
        )),
        "gdelt_actor_pair" | "gdelt-actor-pair" => Some(AlgorithmicLens::gdelt_actor_pair(
            &spec.name,
            spec.modality,
            sparse_dim(spec.output)?,
        )),
        "gdelt_event_actor" | "gdelt-event-actor" => Some(AlgorithmicLens::gdelt_event_actor(
            &spec.name,
            spec.modality,
            sparse_dim(spec.output)?,
        )),
        "gdelt_tone_signal" | "gdelt-tone-signal" => Some(AlgorithmicLens::gdelt_tone_signal(
            &spec.name,
            spec.modality,
            sparse_dim(spec.output)?,
        )),
        "gdelt_source_event" | "gdelt-source-event" => Some(AlgorithmicLens::gdelt_source_event(
            &spec.name,
            spec.modality,
            sparse_dim(spec.output)?,
        )),
        "sparse" | "sparse_keywords" | "sparse-keywords" => Some(AlgorithmicLens::sparse_keywords(
            &spec.name,
            spec.modality,
            sparse_dim(spec.output)?,
        )),
        "token_hash" | "token-hash" | "multi_hash" | "multi-hash" => Some(
            AlgorithmicLens::token_hash(&spec.name, spec.modality, token_dim(spec.output)?),
        ),
        "one_hot" | "one-hot" => Some(AlgorithmicLens::one_hot(
            &spec.name,
            spec.modality,
            dense_dim(spec.output)?,
        )),
        value => {
            if let Some(dim) = value
                .strip_prefix("sparse_keywords:")
                .or_else(|| value.strip_prefix("sparse-keywords:"))
                .and_then(|dim| dim.parse().ok())
            {
                return Some(AlgorithmicLens::sparse_keywords(
                    &spec.name,
                    spec.modality,
                    dim,
                ));
            }
            if let Some(dim) = value
                .strip_prefix("token_hash:")
                .or_else(|| value.strip_prefix("token-hash:"))
                .or_else(|| value.strip_prefix("multi_hash:"))
                .or_else(|| value.strip_prefix("multi-hash:"))
                .and_then(|dim| dim.parse().ok())
            {
                return Some(AlgorithmicLens::token_hash(&spec.name, spec.modality, dim));
            }
            value
                .strip_prefix("one_hot:")
                .or_else(|| value.strip_prefix("one-hot:"))
                .and_then(|buckets| buckets.parse().ok())
                .map(|buckets| AlgorithmicLens::one_hot(&spec.name, spec.modality, buckets))
        }
    }
}

fn dense_dim(shape: SlotShape) -> Option<u32> {
    match shape {
        SlotShape::Dense(dim) => Some(dim),
        SlotShape::Sparse(_) | SlotShape::Multi { .. } => None,
    }
}

fn sparse_dim(shape: SlotShape) -> Option<u32> {
    match shape {
        SlotShape::Sparse(dim) => Some(dim),
        SlotShape::Dense(_) | SlotShape::Multi { .. } => None,
    }
}

fn token_dim(shape: SlotShape) -> Option<u32> {
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
