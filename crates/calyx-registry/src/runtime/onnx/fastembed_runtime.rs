use std::path::{Path, PathBuf};
use std::str::FromStr;

use calyx_core::{CalyxError, Modality, Result, SlotShape};
use fastembed::{EmbeddingModel, TextEmbedding, TextInitOptions};
use hf_hub::api::sync::ApiBuilder;
use ort::ep;

use super::cuda_guard::CudaDropGuard;
use super::{OnnxLens, OnnxModelFiles, OnnxProviderPolicy};
use crate::frozen::{FrozenLensContract, LensDType, NormPolicy, sha256_digest};
use crate::runtime::common::{default_hf_cache_root, fastembed_cache_root, hash_files};

pub fn default_cache_root() -> PathBuf {
    default_hf_cache_root()
}

pub fn from_hf_cache(name: impl Into<String>, cache_dir: PathBuf) -> Result<OnnxLens> {
    from_hf_cache_with_policy(name, cache_dir, OnnxProviderPolicy::CudaFailLoud)
}

pub fn from_hf_cache_with_policy(
    name: impl Into<String>,
    cache_dir: PathBuf,
    provider_policy: OnnxProviderPolicy,
) -> Result<OnnxLens> {
    from_model_with_policy(
        name,
        EmbeddingModel::AllMiniLML6V2,
        cache_dir,
        provider_policy,
    )
}

pub fn from_model_with_policy(
    name: impl Into<String>,
    model_name: EmbeddingModel,
    cache_dir: PathBuf,
    provider_policy: OnnxProviderPolicy,
) -> Result<OnnxLens> {
    let _ort_dylib = super::dynamic_ort::ensure_dynamic_ort()?;
    let name = name.into();
    let info = TextEmbedding::get_model_info(&model_name).map_err(|err| {
        CalyxError::lens_unreachable(format!("fastembed model metadata failed: {err}"))
    })?;
    let model = TextEmbedding::try_new(
        TextInitOptions::new(model_name.clone())
            .with_cache_dir(cache_dir.clone())
            .with_show_download_progress(false)
            .with_intra_threads(1)
            .with_execution_providers(execution_providers(provider_policy)),
    )
    .map_err(|err| CalyxError::lens_unreachable(format!("ONNX runtime init failed: {err}")))?;
    let model = CudaDropGuard::new(model, provider_policy);
    let effective_cache = fastembed_cache_root(&cache_dir);
    let files = resolve_files(
        &effective_cache,
        &info.model_code,
        &info.model_file,
        &info.additional_files,
    )?;
    let weights_sha256 = hash_files(&files.artifact_paths())?;
    let corpus_hash = sha256_digest(&[
        b"onnx-fastembed-mean-pool-v1",
        info.model_code.as_bytes(),
        info.model_file.as_bytes(),
    ]);
    let dim = u32::try_from(info.dim)
        .map_err(|_| CalyxError::lens_dim_mismatch(format!("ONNX dim {} exceeds u32", info.dim)))?;
    let contract = FrozenLensContract::new(
        name,
        weights_sha256,
        corpus_hash,
        SlotShape::Dense(dim),
        Modality::Text,
        LensDType::F32,
        NormPolicy::unit(),
    );
    let id = contract.lens_id();
    Ok(OnnxLens::from_fastembed_parts(
        id,
        dim,
        contract,
        files,
        provider_policy,
        model.into_inner(),
    ))
}

pub fn from_model_name_with_policy(
    name: impl Into<String>,
    model_name: &str,
    cache_dir: PathBuf,
    provider_policy: OnnxProviderPolicy,
) -> Result<OnnxLens> {
    let model_name = model_from_name(model_name)?;
    from_model_with_policy(name, model_name, cache_dir, provider_policy)
}

pub fn model_from_name(raw: &str) -> Result<EmbeddingModel> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(CalyxError::lens_unreachable(
            "fastembed model name must not be empty",
        ));
    }
    if let Ok(model) = EmbeddingModel::from_str(trimmed) {
        return Ok(model);
    }
    match normalized(trimmed).as_str() {
        "baai/bge-m3" | "bge-m3" => Ok(EmbeddingModel::BGEM3),
        "baai/bge-base-en-v1.5" | "xenova/bge-base-en-v1.5" | "bge-base-en-v1.5" => {
            Ok(EmbeddingModel::BGEBaseENV15)
        }
        "qdrant/bge-base-en-v1.5-onnx-q" | "bge-base-en-v1.5-q" => {
            Ok(EmbeddingModel::BGEBaseENV15Q)
        }
        "nomic-ai/nomic-embed-text-v1.5" | "nomic-embed-text-v1.5" => {
            Ok(EmbeddingModel::NomicEmbedTextV15)
        }
        "nomic-ai/nomic-embed-text-v1.5-q" | "nomic-embed-text-v1.5-q" => {
            Ok(EmbeddingModel::NomicEmbedTextV15Q)
        }
        "intfloat/multilingual-e5-base" | "multilingual-e5-base" => {
            Ok(EmbeddingModel::MultilingualE5Base)
        }
        "jinaai/jina-embeddings-v2-base-en" | "jina-embeddings-v2-base-en" => {
            Ok(EmbeddingModel::JinaEmbeddingsV2BaseEN)
        }
        "jinaai/jina-embeddings-v2-base-code" | "jina-embeddings-v2-base-code" | "jina-code" => {
            Ok(EmbeddingModel::JinaEmbeddingsV2BaseCode)
        }
        "google/embeddinggemma-300m"
        | "onnx-community/embeddinggemma-300m-onnx"
        | "embeddinggemma-300m"
        | "embedding-gemma-300m" => Ok(EmbeddingModel::EmbeddingGemma300M),
        "google/embeddinggemma-300m-q4" | "embeddinggemma-300m-q4" => {
            Ok(EmbeddingModel::EmbeddingGemma300MQ4)
        }
        "google/embeddinggemma-300m-q" | "embeddinggemma-300m-q" => {
            Ok(EmbeddingModel::EmbeddingGemma300MQ)
        }
        "snowflake/snowflake-arctic-embed-m" | "snowflake-arctic-embed-m" => {
            Ok(EmbeddingModel::SnowflakeArcticEmbedM)
        }
        "snowflake/snowflake-arctic-embed-m-q" | "snowflake-arctic-embed-m-q" => {
            Ok(EmbeddingModel::SnowflakeArcticEmbedMQ)
        }
        "alibaba-nlp/gte-base-en-v1.5" | "gte-base-en-v1.5" => Ok(EmbeddingModel::GTEBaseENV15),
        other => Err(CalyxError::lens_unreachable(format!(
            "unsupported fastembed model {other}; use a fastembed EmbeddingModel enum name or a supported HF repo id"
        ))),
    }
}

pub fn execution_providers(
    policy: OnnxProviderPolicy,
) -> Vec<fastembed::ExecutionProviderDispatch> {
    match policy {
        // Keep ORT's default kNextPowerOfTwo arena: it pre-grows a pooled block
        // that absorbs the large, variable transient attention buffers real
        // (long) clinical sequences demand. SameAsRequested was tried to shrink
        // resident VRAM but fragmented the arena and failed to allocate a 256 MiB
        // attention buffer mid-ingest on long PubMedQA inputs. VRAM is instead
        // controlled by sizing the panel (fewer, less-redundant lenses), which
        // does not risk a hard OOM on a sequence-length spike.
        OnnxProviderPolicy::CudaFailLoud => vec![
            ep::CUDA::default()
                .with_device_id(0)
                .build()
                .error_on_failure(),
        ],
        OnnxProviderPolicy::CpuExplicit => vec![ep::CPU::default().build()],
    }
}

pub(super) fn resolve_files(
    cache_dir: &Path,
    model_code: &str,
    model_file: &str,
    additional_files: &[String],
) -> Result<OnnxModelFiles> {
    let api = ApiBuilder::new()
        .with_cache_dir(cache_dir.to_path_buf())
        .with_progress(false)
        .build()
        .map_err(|err| CalyxError::lens_unreachable(format!("HF API init failed: {err}")))?;
    let repo = api.model(model_code.to_string());
    let model_file = fetch(&repo, model_file)?;
    let tokenizer = fetch(&repo, "tokenizer.json")?;
    let config = fetch(&repo, "config.json")?;
    let special_tokens_map = fetch(&repo, "special_tokens_map.json")?;
    let tokenizer_config = fetch(&repo, "tokenizer_config.json")?;
    let mut contract_paths = vec![
        model_file.clone(),
        tokenizer.clone(),
        config.clone(),
        tokenizer_config.clone(),
        special_tokens_map.clone(),
    ];
    for file in additional_files {
        contract_paths.push(fetch(&repo, file)?);
    }
    Ok(OnnxModelFiles {
        cache_dir: cache_dir.to_path_buf(),
        model_code: model_code.to_string(),
        model_file,
        tokenizer,
        config,
        special_tokens_map,
        tokenizer_config,
        contract_paths,
    })
}

pub(super) fn fetch(repo: &hf_hub::api::sync::ApiRepo, filename: &str) -> Result<PathBuf> {
    repo.get(filename)
        .map_err(|err| CalyxError::lens_unreachable(format!("fetch {filename} failed: {err}")))
}

fn normalized(raw: &str) -> String {
    raw.trim().to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roster_aliases_resolve_to_fastembed_dense_models() {
        assert!(matches!(
            model_from_name("BAAI/bge-base-en-v1.5").unwrap(),
            EmbeddingModel::BGEBaseENV15
        ));
        assert!(matches!(
            model_from_name("jinaai/jina-embeddings-v2-base-code").unwrap(),
            EmbeddingModel::JinaEmbeddingsV2BaseCode
        ));
        assert!(matches!(
            model_from_name("google/embeddinggemma-300m").unwrap(),
            EmbeddingModel::EmbeddingGemma300M
        ));
    }

    #[test]
    fn roster_quantized_aliases_resolve_to_explicit_variants() {
        assert!(matches!(
            model_from_name("bge-base-en-v1.5-q").unwrap(),
            EmbeddingModel::BGEBaseENV15Q
        ));
        assert!(matches!(
            model_from_name("embeddinggemma-300m-q4").unwrap(),
            EmbeddingModel::EmbeddingGemma300MQ4
        ));
        assert!(matches!(
            model_from_name("embeddinggemma-300m-q").unwrap(),
            EmbeddingModel::EmbeddingGemma300MQ
        ));
    }
}
