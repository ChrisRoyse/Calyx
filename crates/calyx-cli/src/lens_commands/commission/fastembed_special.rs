use std::fs;
use std::path::Path;

use calyx_core::{Input, Lens, Modality, SlotShape};
use calyx_registry::{
    CandleDevicePolicy, CandlePrecision, FastembedBgem3Lens, FastembedBgem3Output,
    FastembedQwen3Lens, FastembedRerankerLens, FastembedSparseLens, NormPolicy, OnnxProviderPolicy,
    Qwen3ModelFiles,
};
use serde_json::json;

use super::artifact::{Artifact, artifact};
use super::fastembed::{FastembedCommission, cache_dir, common_root, copy_artifacts};
use super::log::ConversionLog;
use super::options::{CommissionFlags, CommissionRuntime};
use crate::error::{CliError, CliResult};
use crate::lens_commands::support::validate_vector_contract;

pub(super) fn commission(
    flags: &CommissionFlags,
    out: &Path,
    log: &mut ConversionLog,
) -> CliResult<FastembedCommission> {
    match flags.runtime {
        CommissionRuntime::FastembedSparse => commission_sparse(flags, out, log),
        CommissionRuntime::FastembedBgem3Dense => {
            commission_bgem3(flags, out, log, FastembedBgem3Output::Dense)
        }
        CommissionRuntime::FastembedBgem3Sparse => {
            commission_bgem3(flags, out, log, FastembedBgem3Output::Sparse)
        }
        CommissionRuntime::FastembedBgem3Colbert => {
            commission_bgem3(flags, out, log, FastembedBgem3Output::Colbert)
        }
        CommissionRuntime::FastembedReranker => commission_reranker(flags, out, log),
        CommissionRuntime::FastembedQwen3 => commission_qwen3(flags, out, log),
        _ => Err(CliError::usage(
            "runtime is not a fastembed special runtime",
        )),
    }
}

fn commission_sparse(
    flags: &CommissionFlags,
    out: &Path,
    log: &mut ConversionLog,
) -> CliResult<FastembedCommission> {
    let lens = FastembedSparseLens::from_model_name_with_policy(
        flags.lens_name(),
        &flags.hf,
        cache_dir(flags)?,
        OnnxProviderPolicy::CudaFailLoud,
    )?;
    let probe = Input::new(
        Modality::Text,
        b"Calyx fastembed sparse commission probe".to_vec(),
    );
    let vector = lens.measure(&probe)?;
    validate_vector_contract(&vector, lens.shape(), norm_policy(flags))?;
    let dim = dim(lens.shape());
    let artifacts = copy_artifacts(lens.files(), out)?;
    log.event(json!({
        "event": "fastembed_sparse_verified",
        "model_code": lens.files().model_code,
        "provider_policy": lens.provider_policy(),
        "dim": dim,
        "artifact_count": artifacts.len(),
    }))?;
    Ok(FastembedCommission { artifacts, dim })
}

fn commission_bgem3(
    flags: &CommissionFlags,
    out: &Path,
    log: &mut ConversionLog,
    output: FastembedBgem3Output,
) -> CliResult<FastembedCommission> {
    let lens = FastembedBgem3Lens::from_model_name_with_policy(
        flags.lens_name(),
        &flags.hf,
        output,
        cache_dir(flags)?,
        OnnxProviderPolicy::CudaFailLoud,
    )?;
    let probe = Input::new(Modality::Text, b"Calyx BGE-M3 commission probe".to_vec());
    let vector = lens.measure(&probe)?;
    validate_vector_contract(&vector, lens.shape(), norm_policy(flags))?;
    let dim = dim(lens.shape());
    let artifacts = copy_artifacts(lens.files(), out)?;
    log.event(json!({
        "event": "fastembed_bgem3_verified",
        "model_code": lens.files().model_code,
        "provider_policy": lens.provider_policy(),
        "runtime": lens.runtime_name(),
        "dim": dim,
        "artifact_count": artifacts.len(),
    }))?;
    Ok(FastembedCommission { artifacts, dim })
}

fn commission_reranker(
    flags: &CommissionFlags,
    out: &Path,
    log: &mut ConversionLog,
) -> CliResult<FastembedCommission> {
    let lens = FastembedRerankerLens::from_model_name_with_policy(
        flags.lens_name(),
        &flags.hf,
        cache_dir(flags)?,
        OnnxProviderPolicy::CudaFailLoud,
    )?;
    let probe = Input::new(
        Modality::Text,
        b"Calyx retrieval query\nCalyx retrieval document".to_vec(),
    );
    let vector = lens.measure(&probe)?;
    validate_vector_contract(&vector, lens.shape(), norm_policy(flags))?;
    let dim = dim(lens.shape());
    let artifacts = copy_artifacts(lens.files(), out)?;
    log.event(json!({
        "event": "fastembed_reranker_verified",
        "model_code": lens.files().model_code,
        "provider_policy": lens.provider_policy(),
        "dim": dim,
        "artifact_count": artifacts.len(),
    }))?;
    Ok(FastembedCommission { artifacts, dim })
}

fn commission_qwen3(
    flags: &CommissionFlags,
    out: &Path,
    log: &mut ConversionLog,
) -> CliResult<FastembedCommission> {
    let lens = FastembedQwen3Lens::from_model_id_with_policy(
        flags.lens_name(),
        &flags.hf,
        cache_dir(flags)?,
        CandleDevicePolicy::CudaFailLoud { ordinal: 0 },
        CandlePrecision::BF16,
    )?;
    let probe = Input::new(Modality::Text, b"Calyx Qwen3 commission probe".to_vec());
    let vector = lens.measure(&probe)?;
    validate_vector_contract(&vector, lens.shape(), norm_policy(flags))?;
    let dim = dim(lens.shape());
    let artifacts = copy_qwen3_artifacts(lens.files(), out)?;
    log.event(json!({
        "event": "fastembed_qwen3_verified",
        "model_id": lens.files().model_id,
        "device_policy": lens.device_policy().as_str(),
        "precision": lens.precision().as_str(),
        "max_tokens": lens.max_tokens(),
        "runtime": lens.runtime_name(),
        "dim": dim,
        "artifact_count": artifacts.len(),
    }))?;
    Ok(FastembedCommission { artifacts, dim })
}

fn norm_policy(flags: &CommissionFlags) -> NormPolicy {
    match flags.manifest_norm().as_str() {
        "l2" | "unit" => NormPolicy::unit(),
        "finite" => NormPolicy::Finite,
        _ => NormPolicy::None,
    }
}

fn dim(shape: SlotShape) -> u32 {
    match shape {
        SlotShape::Dense(dim) | SlotShape::Sparse(dim) => dim,
        SlotShape::Multi { token_dim } => token_dim,
    }
}

fn copy_qwen3_artifacts(files: &Qwen3ModelFiles, out: &Path) -> CliResult<Vec<Artifact>> {
    let sources = files.artifact_paths();
    let root = common_root(&sources)?;
    let dest_root = out.join("fastembed-qwen3-artifacts");
    let mut artifacts = Vec::with_capacity(sources.len());
    for source in sources {
        let relative = source.strip_prefix(&root).map_err(|_| {
            CliError::usage(format!(
                "Qwen3 artifact {} is outside common root {}",
                source.display(),
                root.display()
            ))
        })?;
        let dest = dest_root.join(relative);
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(&source, &dest)?;
        artifacts.push(artifact(files.role_for_path(&source), dest)?);
    }
    Ok(artifacts)
}
