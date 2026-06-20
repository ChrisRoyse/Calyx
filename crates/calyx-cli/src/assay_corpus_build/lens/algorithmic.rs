use calyx_core::Lens;
use calyx_registry::{AlgorithmicLens, LensSpec as RegistryLensSpec};

use crate::lens_commands::support::dim;

pub(super) fn algorithmic_lens(
    spec: &RegistryLensSpec,
    kind: &str,
) -> Result<AlgorithmicLens, String> {
    let lens = match kind {
        "byte" | "byte-features" | "byte_features" => {
            AlgorithmicLens::byte_features(&spec.name, spec.modality)
        }
        "scalar" => AlgorithmicLens::scalar(&spec.name, spec.modality),
        "ast-style" | "ast_style" => AlgorithmicLens::ast_style(&spec.name, spec.modality),
        "gdelt-cameo" | "gdelt_cameo" => AlgorithmicLens::gdelt_cameo(&spec.name, spec.modality),
        "gdelt-actor-geo" | "gdelt_actor_geo" => {
            AlgorithmicLens::gdelt_actor_geo(&spec.name, spec.modality, dim(spec.output))
        }
        value if value.starts_with("one-hot:") || value.starts_with("one_hot:") => {
            let buckets = parse_kind_dim(value)?;
            AlgorithmicLens::one_hot(&spec.name, spec.modality, buckets)
        }
        "sparse" | "sparse-keywords" | "sparse_keywords" => {
            AlgorithmicLens::sparse_keywords(&spec.name, spec.modality, dim(spec.output))
        }
        value if value.starts_with("sparse-keywords:") || value.starts_with("sparse_keywords:") => {
            let parsed = parse_kind_dim(value)?;
            AlgorithmicLens::sparse_keywords(&spec.name, spec.modality, parsed)
        }
        "token-hash" | "token_hash" | "multi-hash" | "multi_hash" => {
            AlgorithmicLens::token_hash(&spec.name, spec.modality, dim(spec.output))
        }
        value
            if value.starts_with("token-hash:")
                || value.starts_with("token_hash:")
                || value.starts_with("multi-hash:")
                || value.starts_with("multi_hash:") =>
        {
            let parsed = parse_kind_dim(value)?;
            AlgorithmicLens::token_hash(&spec.name, spec.modality, parsed)
        }
        other => {
            return Err(format!(
                "CALYX_FSV_ASSAY_CORPUS_BUILD_UNSUPPORTED_ALGORITHMIC_LENS: lens={} kind={other}",
                spec.name
            ));
        }
    };
    if lens.shape() != spec.output {
        return Err(format!(
            "CALYX_FSV_ASSAY_CORPUS_BUILD_ALGORITHMIC_SHAPE_MISMATCH: lens={} runtime_shape={:?} manifest_shape={:?}",
            spec.name,
            lens.shape(),
            spec.output
        ));
    }
    Ok(lens)
}

fn parse_kind_dim(kind: &str) -> Result<u32, String> {
    kind.split_once(':')
        .and_then(|(_, value)| value.parse::<u32>().ok())
        .filter(|dim| *dim > 0)
        .ok_or_else(|| format!("CALYX_FSV_ASSAY_CORPUS_BUILD_INVALID_ALGORITHMIC_DIM: kind={kind}"))
}
