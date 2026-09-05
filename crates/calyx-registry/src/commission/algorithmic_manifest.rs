use calyx_core::{CalyxError, Lens, Modality, Result, SlotShape};

use crate::AlgorithmicLens;
use crate::frozen::FrozenLensContract;

const CONFIG_INVALID: &str = "CALYX_LENS_CONFIG_INVALID";

pub(super) fn is_algorithmic_runtime(runtime: &str) -> bool {
    runtime == "algorithmic" || runtime.starts_with("algorithmic:")
}

pub(super) fn algorithmic_kind(runtime: &str) -> Option<&str> {
    if runtime == "algorithmic" {
        Some("byte-features")
    } else {
        runtime.strip_prefix("algorithmic:")
    }
}

pub(super) fn output_shape(runtime: &str, dim: u32) -> Result<SlotShape> {
    let Some(kind) = algorithmic_kind(runtime) else {
        return learned_output_shape(runtime, dim);
    };
    AlgorithmicLens::from_runtime_kind_dim(
        "algorithmic-shape-probe",
        Modality::Structured,
        kind,
        dim,
    )
    .map(|lens| lens.shape())
    .map_err(|error| config_invalid(error.message))
}

pub(super) fn frozen_contract(
    name: &str,
    runtime: &str,
    modality: Modality,
    shape: SlotShape,
) -> Result<Option<FrozenLensContract>> {
    let Some(kind) = algorithmic_kind(runtime) else {
        return Ok(None);
    };
    Ok(Some(
        AlgorithmicLens::from_runtime_kind(name, modality, kind, shape)
            .map_err(|error| config_invalid(error.message))?
            .contract()
            .clone(),
    ))
}

fn learned_output_shape(runtime: &str, dim: u32) -> Result<SlotShape> {
    match runtime {
        "fastembed-sparse" | "fastembed-bgem3-sparse" | "onnx-bgem3-sparse" | "onnx-splade" => {
            Ok(SlotShape::Sparse(checked_positive(runtime, dim)?))
        }
        "fastembed-bgem3-colbert" | "onnx-bgem3-colbert" | "onnx-colbert" => Ok(SlotShape::Multi {
            token_dim: checked_positive(runtime, dim)?,
        }),
        "tei-segmented" | "tei-http-segmented" | "tei_http_segmented" => Ok(SlotShape::Multi {
            token_dim: checked_positive(runtime, dim)?,
        }),
        _ => Ok(SlotShape::Dense(dim)),
    }
}

fn checked_positive(kind: &str, dim: u32) -> Result<u32> {
    if dim > 0 {
        return Ok(dim);
    }
    Err(config_invalid(format!(
        "algorithmic lens {kind} dim must be greater than zero"
    )))
}

fn config_invalid(message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: CONFIG_INVALID,
        message: message.into(),
        remediation: "fix the lensforge manifest or regenerated artifacts",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gdelt_algorithmic_shapes_are_explicit() {
        assert_eq!(
            output_shape("algorithmic:gdelt-cameo", 16).unwrap(),
            SlotShape::Dense(16)
        );
        assert_eq!(
            output_shape("algorithmic:gdelt-actor-geo", 512).unwrap(),
            SlotShape::Sparse(512)
        );
        assert_eq!(
            output_shape("algorithmic:gdelt-source-event", 512).unwrap(),
            SlotShape::Sparse(512)
        );
    }

    #[test]
    fn gdelt_cameo_rejects_wrong_manifest_dim() {
        let error = output_shape("algorithmic:gdelt-cameo", 8).unwrap_err();

        // gdelt-cameo has a fixed dimension, so a manifest requesting 8 must be
        // rejected. Assert the kind is named and the rejected dimension (8) is
        // reported, without pinning the exact prose.
        assert_eq!(error.code, CONFIG_INVALID);
        assert!(error.message.contains("gdelt-cameo"), "{}", error.message);
        assert!(error.message.contains('8'), "{}", error.message);
    }

    #[test]
    fn gdelt_actor_geo_rejects_zero_manifest_dim() {
        let error = output_shape("algorithmic:gdelt-actor-geo", 0).unwrap_err();

        assert_eq!(error.code, CONFIG_INVALID);
        assert!(
            error.message.contains("gdelt-actor-geo"),
            "{}",
            error.message
        );
        assert!(
            error.message.contains("non-zero dimension"),
            "{}",
            error.message
        );
    }

    #[test]
    fn sparse_algorithmic_kinds_reject_zero_manifest_dim() {
        let error = output_shape("algorithmic:sparse-keywords", 0).unwrap_err();

        assert_eq!(error.code, CONFIG_INVALID);
        assert!(
            error.message.contains("sparse-keywords"),
            "{}",
            error.message
        );
        assert!(
            error.message.contains("non-zero dimension"),
            "{}",
            error.message
        );
    }

    #[test]
    fn token_algorithmic_kinds_reject_zero_manifest_dim() {
        let error = output_shape("algorithmic:token-hash", 0).unwrap_err();

        assert_eq!(error.code, CONFIG_INVALID);
        assert!(error.message.contains("token-hash"), "{}", error.message);
        assert!(
            error.message.contains("non-zero dimension"),
            "{}",
            error.message
        );
    }

    #[test]
    fn learned_special_runtimes_declare_non_dense_shapes() {
        assert_eq!(
            output_shape("fastembed-sparse", 30_522).unwrap(),
            SlotShape::Sparse(30_522)
        );
        assert_eq!(
            output_shape("fastembed-bgem3-sparse", 250_002).unwrap(),
            SlotShape::Sparse(250_002)
        );
        assert_eq!(
            output_shape("onnx-splade", 30_522).unwrap(),
            SlotShape::Sparse(30_522)
        );
        assert_eq!(
            output_shape("fastembed-bgem3-colbert", 1024).unwrap(),
            SlotShape::Multi { token_dim: 1024 }
        );
        assert_eq!(
            output_shape("onnx-colbert", 96).unwrap(),
            SlotShape::Multi { token_dim: 96 }
        );
    }
}
