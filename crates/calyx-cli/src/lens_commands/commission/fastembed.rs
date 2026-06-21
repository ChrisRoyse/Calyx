use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use calyx_core::{Input, Lens, Modality, SlotShape};
use calyx_registry::{NormPolicy, OnnxLens, OnnxModelFiles, OnnxProviderPolicy};
use serde_json::json;

use super::artifact::{Artifact, artifact};
use super::log::ConversionLog;
use super::options::CommissionFlags;
use crate::error::{CliError, CliResult};
use crate::lens_commands::support::validate_vector_contract;

pub(super) struct FastembedCommission {
    pub(super) artifacts: Vec<Artifact>,
    pub(super) dim: u32,
}

pub(super) fn commission(
    flags: &CommissionFlags,
    out: &Path,
    log: &mut ConversionLog,
) -> CliResult<FastembedCommission> {
    let lens = OnnxLens::from_model_name_with_policy(
        flags.lens_name(),
        &flags.hf,
        cache_dir(flags)?,
        OnnxProviderPolicy::CudaFailLoud,
    )?;
    let probe = Input::new(
        Modality::Text,
        b"Calyx fastembed ONNX commission probe".to_vec(),
    );
    let vector = lens.measure(&probe)?;
    validate_vector_contract(&vector, lens.shape(), NormPolicy::unit())?;
    let dim = dense_dim(lens.shape())?;
    let artifacts = copy_artifacts(lens.files(), out)?;
    log.event(json!({
        "event": "fastembed_onnx_verified",
        "model_code": lens.files().model_code,
        "provider_policy": lens.provider_policy(),
        "runtime": lens.runtime_name(),
        "dim": dim,
        "artifact_count": artifacts.len(),
    }))?;
    Ok(FastembedCommission { artifacts, dim })
}

pub(super) fn cache_dir(flags: &CommissionFlags) -> CliResult<PathBuf> {
    if let Some(home) = &flags.home {
        return Ok(home.join(".hf-cache"));
    }
    if let Some(path) = env::var_os("HF_HOME") {
        return Ok(path.into());
    }
    env::var_os("CALYX_HOME")
        .map(PathBuf::from)
        .map(|home| home.join(".hf-cache"))
        .ok_or_else(|| {
            CliError::usage("CALYX_HOME, HF_HOME, or --home is required for fastembed-onnx")
        })
}

pub(super) fn copy_artifacts(files: &OnnxModelFiles, out: &Path) -> CliResult<Vec<Artifact>> {
    let sources = files.artifact_paths();
    let root = common_root(&sources)?;
    copy_artifacts_from_root(files, out, &root, "fastembed-artifacts")
}

pub(super) fn copy_artifacts_into(
    files: &OnnxModelFiles,
    out: &Path,
    dirname: &str,
) -> CliResult<Vec<Artifact>> {
    let sources = files.artifact_paths();
    let root = common_root(&sources)?;
    copy_artifacts_from_root(files, out, &root, dirname)
}

fn copy_artifacts_from_root(
    files: &OnnxModelFiles,
    out: &Path,
    root: &Path,
    dirname: &str,
) -> CliResult<Vec<Artifact>> {
    let sources = files.artifact_paths();
    let dest_root = out.join(dirname);
    let mut artifacts = Vec::with_capacity(sources.len());
    for source in sources {
        let relative = source.strip_prefix(root).map_err(|_| {
            CliError::usage(format!(
                "fastembed artifact {} is outside common root {}",
                source.display(),
                root.display()
            ))
        })?;
        let dest = dest_root.join(relative);
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(&source, &dest)?;
        artifacts.push(artifact(role_for(&source, files), dest)?);
    }
    Ok(artifacts)
}

pub(super) fn common_root(paths: &[PathBuf]) -> CliResult<PathBuf> {
    let first = paths
        .first()
        .ok_or_else(|| CliError::usage("fastembed produced no artifact files"))?;
    let mut root = first
        .parent()
        .ok_or_else(|| CliError::usage("fastembed artifact has no parent"))?
        .to_path_buf();
    for path in &paths[1..] {
        while !path.starts_with(&root) {
            root = root
                .parent()
                .ok_or_else(|| CliError::usage("fastembed artifacts share no common root"))?
                .to_path_buf();
        }
    }
    Ok(root)
}

fn role_for(source: &Path, files: &OnnxModelFiles) -> &'static str {
    if source == files.model_file.as_path() {
        "model"
    } else if source == files.tokenizer.as_path() {
        "tokenizer"
    } else if source == files.config.as_path() {
        "config"
    } else if source == files.tokenizer_config.as_path() {
        "tokenizer_config"
    } else if source == files.special_tokens_map.as_path() {
        "special_tokens_map"
    } else {
        "model_sidecar"
    }
}

fn dense_dim(shape: SlotShape) -> CliResult<u32> {
    match shape {
        SlotShape::Dense(dim) => Ok(dim),
        other => Err(CliError::usage(format!(
            "fastembed-onnx expected dense output, got {other:?}"
        ))),
    }
}
