use std::path::PathBuf;

use calyx_core::{CalyxError, Result};

use super::algorithmic_manifest::algorithmic_kind;
use super::manifest::{LensForgeManifest, VerifiedFile, modality_token};
use crate::runtime::qwen3::DEFAULT_QWEN3_MAX_TOKENS;
use crate::runtime::tei_segmented::{
    DEFAULT_TEI_SEGMENTATION_POLICY, TeiSegmentationPolicy, pooled_paragraph_policy,
    segments_paragraph_policy,
};
use crate::spec::{Bgem3Engine, FastembedBgem3Output, LensRuntime};

pub(super) fn runtime_from_manifest(
    manifest: &LensForgeManifest,
    artifacts: &[VerifiedFile],
) -> Result<LensRuntime> {
    if let Some(kind) = algorithmic_kind(&manifest.runtime) {
        return Ok(LensRuntime::Algorithmic {
            kind: kind.to_string(),
        });
    }
    let files = artifacts
        .iter()
        .map(|file| file.path.clone())
        .collect::<Vec<_>>();
    match manifest.runtime.as_str() {
        "onnx" | "onnx-int8" | "onnx-custom" | "onnx-fastembed" | "onnx-splade" => {
            Ok(LensRuntime::Onnx {
                model_id: manifest.source_hf_id.clone(),
                files,
            })
        }
        "onnx-colbert" => Ok(LensRuntime::OnnxColbert {
            model_id: manifest.source_hf_id.clone(),
            files,
        }),
        "fastembed-sparse" => Ok(LensRuntime::FastembedSparse {
            model_id: manifest.source_hf_id.clone(),
            files,
        }),
        "fastembed-bgem3-dense" => Ok(fastembed_bgem3_runtime(
            manifest,
            files,
            FastembedBgem3Output::Dense,
            Bgem3Engine::FastembedCpu,
        )),
        "fastembed-bgem3-sparse" => Ok(fastembed_bgem3_runtime(
            manifest,
            files,
            FastembedBgem3Output::Sparse,
            Bgem3Engine::FastembedCpu,
        )),
        "fastembed-bgem3-colbert" => Ok(fastembed_bgem3_runtime(
            manifest,
            files,
            FastembedBgem3Output::Colbert,
            Bgem3Engine::FastembedCpu,
        )),
        "onnx-bgem3-dense" => Ok(fastembed_bgem3_runtime(
            manifest,
            files,
            FastembedBgem3Output::Dense,
            Bgem3Engine::OnnxCuda,
        )),
        "onnx-bgem3-sparse" => Ok(fastembed_bgem3_runtime(
            manifest,
            files,
            FastembedBgem3Output::Sparse,
            Bgem3Engine::OnnxCuda,
        )),
        "onnx-bgem3-colbert" => Ok(fastembed_bgem3_runtime(
            manifest,
            files,
            FastembedBgem3Output::Colbert,
            Bgem3Engine::OnnxCuda,
        )),
        "fastembed-reranker" => Ok(LensRuntime::FastembedReranker {
            model_id: manifest.source_hf_id.clone(),
            files,
        }),
        "fastembed-qwen3" => Ok(LensRuntime::FastembedQwen3 {
            model_id: manifest.source_hf_id.clone(),
            files,
            dtype: manifest.dtype.clone(),
            max_tokens: manifest.max_tokens.unwrap_or(DEFAULT_QWEN3_MAX_TOKENS),
        }),
        "candle" | "candle-fp16" | "candle-local" => Ok(LensRuntime::CandleLocal {
            model_id: manifest.source_hf_id.clone(),
            files,
            dtype: manifest.dtype.clone(),
            pooling: manifest.pooling.clone(),
        }),
        "tei" | "tei-http" | "tei_http" => Ok(LensRuntime::TeiHttp {
            endpoint: manifest
                .endpoint
                .clone()
                .ok_or_else(|| config_invalid("lensforge TEI endpoint is required"))?,
        }),
        "tei-segmented" | "tei-http-segmented" | "tei_http_segmented" => {
            Ok(LensRuntime::TeiHttpSegmented {
                endpoint: manifest.endpoint.clone().ok_or_else(|| {
                    config_invalid("lensforge segmented TEI endpoint is required")
                })?,
                policy: segmented_tei_policy(manifest)?,
            })
        }
        "model2vec" | "static_lookup" | "static-lookup" => Ok(LensRuntime::StaticLookup {
            embeddings_file: artifact_by_role(artifacts, is_model_role)?,
            tokenizer: artifact_by_role(artifacts, |role| role == "tokenizer")?,
            dim: manifest.dim,
        }),
        "external-cmd" | "external_cmd" => Ok(LensRuntime::ExternalCmd {
            cmd: manifest.source_hf_id.clone(),
            args: artifact_args(artifacts),
        }),
        "adapter" | "multimodal-adapter" | "multimodal_adapter" => {
            let adapter_config = artifact_by_role(artifacts, |role| role == "adapter")?;
            Ok(LensRuntime::MultimodalAdapter {
                axis: modality_token(manifest.modality).to_string(),
                model_id: manifest.source_hf_id.clone(),
                adapter_config: Some(adapter_config),
                files,
            })
        }
        "model2vec-external" => Ok(LensRuntime::ExternalCmd {
            cmd: "model2vec".to_string(),
            args: files
                .iter()
                .map(|path| path.display().to_string())
                .collect(),
        }),
        other => Err(config_invalid(format!(
            "unsupported lensforge runtime {other}"
        ))),
    }
}

/// Builds the frozen segmentation policy for a segmented TEI lens from the
/// manifest. The per-segment window is carried by the existing `max_tokens`
/// field (which is already hashed into the lens `corpus_hash`) and must match
/// the router's `max_input_length`; the remaining policy terms are the frozen
/// defaults. Fails closed if the window is absent or the policy is invalid.
pub(super) fn segmented_tei_policy(manifest: &LensForgeManifest) -> Result<TeiSegmentationPolicy> {
    let max_tokens = manifest.max_tokens.ok_or_else(|| {
        config_invalid(
            "segmented TEI lensforge manifest requires max_tokens (the frozen per-segment window matching the router max_input_length)",
        )
    })?;
    let max_input_tokens = u32::try_from(max_tokens)
        .map_err(|_| config_invalid("segmented TEI manifest max_tokens exceeds u32"))?;
    let policy = match manifest.representation.as_deref() {
        None | Some("segments") | Some("preserve-segments") => TeiSegmentationPolicy {
            max_input_tokens,
            ..DEFAULT_TEI_SEGMENTATION_POLICY
        },
        Some("pooled-paragraph") | Some("pooled") => pooled_paragraph_policy(max_input_tokens),
        Some("segments-paragraph") => segments_paragraph_policy(max_input_tokens),
        Some(other) => {
            return Err(config_invalid(format!(
                "unsupported segmented TEI representation {other:?}; expected \"segments\", \"segments-paragraph\" or \"pooled-paragraph\""
            )));
        }
    };
    let mut policy = policy;
    if let Some(special_tokens) = manifest.special_tokens {
        policy.special_tokens = special_tokens;
    }
    policy.validate()?;
    Ok(policy)
}

fn fastembed_bgem3_runtime(
    manifest: &LensForgeManifest,
    files: Vec<PathBuf>,
    output: FastembedBgem3Output,
    engine: Bgem3Engine,
) -> LensRuntime {
    LensRuntime::FastembedBgem3 {
        model_id: manifest.source_hf_id.clone(),
        files,
        output,
        engine,
    }
}

fn artifact_args(artifacts: &[VerifiedFile]) -> Vec<String> {
    artifacts
        .iter()
        .map(|file| file.path.display().to_string())
        .collect()
}

fn artifact_by_role(
    artifacts: &[VerifiedFile],
    predicate: impl Fn(&str) -> bool,
) -> Result<PathBuf> {
    artifacts
        .iter()
        .find(|file| predicate(&file.role))
        .map(|file| file.path.clone())
        .ok_or_else(|| config_invalid("lensforge manifest missing static lookup artifact"))
}

fn is_model_role(role: &str) -> bool {
    matches!(role, "model" | "weights" | "embeddings")
}

fn config_invalid(message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: "CALYX_LENS_CONFIG_INVALID",
        message: message.into(),
        remediation: "fix the lensforge manifest or regenerated artifacts",
    }
}
