use std::path::{Path, PathBuf};

use calyx_core::{CalyxError, Modality, Result, SlotShape};
use fastembed::{EmbeddingModel, TextEmbedding, TextInitOptions};
use hf_hub::api::sync::ApiBuilder;
use ort::ep;

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
    let effective_cache = fastembed_cache_root(&cache_dir);
    let files = resolve_files(&effective_cache, &info.model_code, &info.model_file)?;
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
        model,
    ))
}

pub fn execution_providers(
    policy: OnnxProviderPolicy,
) -> Vec<fastembed::ExecutionProviderDispatch> {
    match policy {
        OnnxProviderPolicy::CudaFailLoud => vec![
            ep::CUDA::default()
                .with_device_id(0)
                .build()
                .error_on_failure(),
        ],
        OnnxProviderPolicy::CpuExplicit => vec![ep::CPU::default().build()],
    }
}

fn resolve_files(cache_dir: &Path, model_code: &str, model_file: &str) -> Result<OnnxModelFiles> {
    let api = ApiBuilder::new()
        .with_cache_dir(cache_dir.to_path_buf())
        .with_progress(false)
        .build()
        .map_err(|err| CalyxError::lens_unreachable(format!("HF API init failed: {err}")))?;
    let repo = api.model(model_code.to_string());
    Ok(OnnxModelFiles {
        cache_dir: cache_dir.to_path_buf(),
        model_code: model_code.to_string(),
        model_file: fetch(&repo, model_file)?,
        tokenizer: fetch(&repo, "tokenizer.json")?,
        config: fetch(&repo, "config.json")?,
        special_tokens_map: fetch(&repo, "special_tokens_map.json")?,
        tokenizer_config: fetch(&repo, "tokenizer_config.json")?,
    })
}

fn fetch(repo: &hf_hub::api::sync::ApiRepo, filename: &str) -> Result<PathBuf> {
    repo.get(filename)
        .map_err(|err| CalyxError::lens_unreachable(format!("fetch {filename} failed: {err}")))
}
