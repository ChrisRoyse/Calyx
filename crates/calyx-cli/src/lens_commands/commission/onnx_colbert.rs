use std::path::Path;

use calyx_core::{Input, Lens, Modality, SlotShape};
use calyx_registry::{NormPolicy, OnnxColbertLens, OnnxProviderPolicy};
use serde_json::json;

use super::fastembed::{FastembedCommission, cache_dir, copy_artifacts_into};
use super::log::ConversionLog;
use super::options::CommissionFlags;
use crate::error::CliResult;
use crate::lens_commands::support::validate_vector_contract;

pub(super) fn commission(
    flags: &CommissionFlags,
    out: &Path,
    log: &mut ConversionLog,
) -> CliResult<FastembedCommission> {
    let lens = OnnxColbertLens::from_model_id_with_policy(
        flags.lens_name(),
        &flags.hf,
        cache_dir(flags)?,
        OnnxProviderPolicy::CudaFailLoud,
    )?;
    let probe = Input::new(Modality::Text, b"Calyx ColBERT commission probe".to_vec());
    let vector = lens.measure(&probe)?;
    validate_vector_contract(&vector, lens.shape(), NormPolicy::Finite)?;
    let SlotShape::Multi { token_dim } = lens.shape() else {
        unreachable!("OnnxColbertLens always emits multi-vector output")
    };
    let artifacts = copy_artifacts_into(lens.files(), out, "onnx-colbert-artifacts")?;
    log.event(json!({
        "event": "onnx_colbert_verified",
        "model_code": lens.files().model_code,
        "provider_policy": lens.provider_policy(),
        "runtime": lens.runtime_name(),
        "token_dim": token_dim,
        "artifact_count": artifacts.len(),
    }))?;
    Ok(FastembedCommission {
        artifacts,
        dim: token_dim,
    })
}
