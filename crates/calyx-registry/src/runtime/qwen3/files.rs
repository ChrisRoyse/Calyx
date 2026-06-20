use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use calyx_core::{CalyxError, Result};
use hf_hub::api::sync::{ApiBuilder, ApiRepo};

use super::{OPTIONAL_QWEN3_FILES, config_invalid};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Qwen3ModelFiles {
    pub cache_dir: PathBuf,
    pub model_id: String,
    pub config: PathBuf,
    pub tokenizer: PathBuf,
    pub weights: Vec<PathBuf>,
    pub contract_paths: Vec<PathBuf>,
}

impl Qwen3ModelFiles {
    pub fn artifact_paths(&self) -> Vec<PathBuf> {
        let mut paths = if self.contract_paths.is_empty() {
            self.required_paths()
        } else {
            self.contract_paths.clone()
        };
        self.sort_paths(&mut paths);
        paths
    }

    pub fn required_paths(&self) -> Vec<PathBuf> {
        let mut paths = self.weights.clone();
        paths.push(self.tokenizer.clone());
        paths.push(self.config.clone());
        self.sort_paths(&mut paths);
        paths
    }

    pub fn role_for_path(&self, path: &Path) -> &'static str {
        if self.weights.iter().any(|weight| weight == path) {
            "model"
        } else if path == self.tokenizer {
            "tokenizer"
        } else if path == self.config {
            "config"
        } else if file_name_eq(path, "tokenizer_config.json") {
            "tokenizer_config"
        } else if file_name_eq(path, "special_tokens_map.json") {
            "special_tokens_map"
        } else {
            "model_sidecar"
        }
    }

    pub fn from_paths(model_id: impl Into<String>, files: Vec<PathBuf>) -> Result<Self> {
        if files.is_empty() {
            return Err(config_invalid("fastembed-qwen3 requires manifest files"));
        }
        let weights = weight_paths(&files)?;
        let tokenizer = required_file(
            &files,
            |path| file_name_eq(path, "tokenizer.json"),
            "tokenizer.json",
        )?;
        let config = config_path(&files)?;
        let cache_dir = common_parent(&files).unwrap_or_else(|| PathBuf::from("."));
        let mut out = Self {
            cache_dir,
            model_id: model_id.into(),
            config,
            tokenizer,
            weights,
            contract_paths: files,
        };
        out.sort_contract_paths();
        Ok(out)
    }

    fn sort_contract_paths(&mut self) {
        let mut paths = std::mem::take(&mut self.contract_paths);
        self.sort_paths(&mut paths);
        self.contract_paths = paths;
    }

    fn sort_paths(&self, paths: &mut [PathBuf]) {
        paths.sort_by_key(|path| (role_rank(self.role_for_path(path)), path.clone()));
    }
}

pub fn fetch_files(cache_dir: &Path, model_id: &str) -> Result<Qwen3ModelFiles> {
    let api = ApiBuilder::new()
        .with_cache_dir(cache_dir.to_path_buf())
        .with_progress(false)
        .build()
        .map_err(|err| CalyxError::lens_unreachable(format!("HF API init failed: {err}")))?;
    let repo = api.model(model_id.to_string());
    let weights = fetch_weights(&repo)?;
    let tokenizer = fetch(&repo, "tokenizer.json")?;
    let config = fetch(&repo, "config.json")?;
    let mut contract_paths = weights.clone();
    contract_paths.push(tokenizer.clone());
    contract_paths.push(config.clone());
    for filename in OPTIONAL_QWEN3_FILES {
        if let Ok(path) = repo.get(filename)
            && !contract_paths.contains(&path)
        {
            contract_paths.push(path);
        }
    }
    let files = Qwen3ModelFiles {
        cache_dir: cache_dir.to_path_buf(),
        model_id: model_id.to_string(),
        config,
        tokenizer,
        weights,
        contract_paths,
    };
    Qwen3ModelFiles::from_paths(model_id.to_string(), files.artifact_paths())
}

fn fetch_weights(repo: &ApiRepo) -> Result<Vec<PathBuf>> {
    if let Ok(path) = repo.get("model.safetensors") {
        return Ok(vec![path]);
    }
    let mut files = Vec::new();
    for idx in 1.. {
        let mut found = None;
        for total in 1..=20 {
            let name = format!("model-{idx:05}-of-{total:05}.safetensors");
            if let Ok(path) = repo.get(&name) {
                found = Some(path);
                break;
            }
        }
        let Some(path) = found else {
            break;
        };
        files.push(path);
    }
    if files.is_empty() {
        return Err(CalyxError::lens_unreachable(
            "fetch model.safetensors or sharded Qwen3 weights failed",
        ));
    }
    Ok(files)
}

fn fetch(repo: &ApiRepo, filename: &str) -> Result<PathBuf> {
    repo.get(filename)
        .map_err(|err| CalyxError::lens_unreachable(format!("fetch {filename} failed: {err}")))
}

fn weight_paths(paths: &[PathBuf]) -> Result<Vec<PathBuf>> {
    let mut weights = paths
        .iter()
        .filter(|path| {
            path.extension()
                .and_then(OsStr::to_str)
                .is_some_and(|value| value.eq_ignore_ascii_case("safetensors"))
        })
        .cloned()
        .collect::<Vec<_>>();
    weights.sort();
    if let Some(index) = weights
        .iter()
        .position(|path| file_name_eq(path, "model.safetensors"))
    {
        let first = weights.remove(index);
        weights.insert(0, first);
    }
    if weights.is_empty() {
        return Err(config_invalid(
            "fastembed-qwen3 requires safetensors weights",
        ));
    }
    Ok(weights)
}

fn config_path(paths: &[PathBuf]) -> Result<PathBuf> {
    paths
        .iter()
        .find(|path| file_name_eq(path, "config.json") && !has_component(path, "1_Pooling"))
        .or_else(|| paths.iter().find(|path| file_name_eq(path, "config.json")))
        .cloned()
        .ok_or_else(|| config_invalid("fastembed-qwen3 requires config.json"))
}

fn required_file(
    paths: &[PathBuf],
    predicate: impl Fn(&Path) -> bool,
    label: &str,
) -> Result<PathBuf> {
    paths
        .iter()
        .find(|path| predicate(path))
        .cloned()
        .ok_or_else(|| config_invalid(format!("fastembed-qwen3 requires {label}")))
}

fn file_name_eq(path: &Path, name: &str) -> bool {
    path.file_name()
        .is_some_and(|value| value == OsStr::new(name))
}

fn has_component(path: &Path, name: &str) -> bool {
    path.components()
        .any(|component| component.as_os_str() == OsStr::new(name))
}

fn common_parent(paths: &[PathBuf]) -> Option<PathBuf> {
    Some(paths.first()?.parent()?.to_path_buf())
}

fn role_rank(role: &str) -> u8 {
    match role {
        "model" | "weights" | "embeddings" => 0,
        "tokenizer" => 1,
        "config" => 2,
        "preprocessor" => 3,
        "tokenizer_config" => 4,
        "special_tokens_map" => 5,
        _ => 9,
    }
}
