use std::path::{Path, PathBuf};

use calyx_core::{CalyxError, Result};
use hf_hub::api::sync::{ApiBuilder, ApiRepo};

use super::OnnxModelFiles;
use super::colbert::DEFAULT_COLBERT_ONNX;

const OPTIONAL_COLBERT_FILES: &[&str] = &[
    "tokenizer_config.json",
    "special_tokens_map.json",
    "onnx_config.json",
];

pub(in crate::runtime::onnx) fn fetch_answerai_colbert_files(
    cache_dir: &Path,
    model_id: &str,
) -> Result<OnnxModelFiles> {
    let api = ApiBuilder::new()
        .with_cache_dir(cache_dir.to_path_buf())
        .with_progress(false)
        .build()
        .map_err(|err| CalyxError::lens_unreachable(format!("HF API init failed: {err}")))?;
    let repo = api.model(model_id.to_string());
    let model = fetch_first(
        &repo,
        &[
            DEFAULT_COLBERT_ONNX,
            "model_int8.onnx",
            "onnx/model_quantized.onnx",
        ],
    )?;
    let tokenizer = fetch(&repo, "tokenizer.json")?;
    let config = fetch(&repo, "config.json")?;
    let mut contract_paths = vec![model.clone(), tokenizer.clone(), config.clone()];
    let mut tokenizer_config = tokenizer.clone();
    let mut special_tokens_map = config.clone();
    for filename in OPTIONAL_COLBERT_FILES {
        if let Ok(path) = repo.get(filename)
            && !contract_paths.contains(&path)
        {
            if *filename == "tokenizer_config.json" {
                tokenizer_config = path.clone();
            } else if *filename == "special_tokens_map.json" {
                special_tokens_map = path.clone();
            }
            contract_paths.push(path);
        }
    }
    Ok(OnnxModelFiles {
        cache_dir: cache_dir.to_path_buf(),
        model_code: model_id.to_string(),
        model_file: model,
        tokenizer,
        config,
        special_tokens_map,
        tokenizer_config,
        contract_paths,
    })
}

fn fetch_first(repo: &ApiRepo, names: &[&str]) -> Result<PathBuf> {
    for name in names {
        if let Ok(path) = repo.get(name) {
            return Ok(path);
        }
    }
    Err(CalyxError::lens_unreachable(format!(
        "fetch one of {} failed",
        names.join(", ")
    )))
}

fn fetch(repo: &ApiRepo, filename: &str) -> Result<PathBuf> {
    repo.get(filename)
        .map_err(|err| CalyxError::lens_unreachable(format!("fetch {filename} failed: {err}")))
}
