use std::path::Path;

use calyx_core::{CalyxError, Result};
use candle_core::Device;
use candle_nn::VarBuilder;
use candle_transformers::models::bert::{BertModel, Config};
use hf_hub::api::sync::ApiBuilder;
use tokenizers::{Tokenizer, TruncationParams};

use super::{CandleDevicePolicy, CandleModelFiles, CandlePrecision};

pub(super) const HALF_CUDA_MIN_LAYER_NORM_EPS: f64 = 1.0e-5;

pub(super) fn fetch_files(cache_dir: &Path, model_id: &str) -> Result<CandleModelFiles> {
    let api = ApiBuilder::new()
        .with_cache_dir(cache_dir.to_path_buf())
        .with_progress(false)
        .build()
        .map_err(|err| CalyxError::lens_unreachable(format!("HF API init failed: {err}")))?;
    let repo = api.model(model_id.to_string());
    let config = repo
        .get("config.json")
        .map_err(|err| CalyxError::lens_unreachable(format!("fetch config.json failed: {err}")))?;
    let tokenizer = repo.get("tokenizer.json").map_err(|err| {
        CalyxError::lens_unreachable(format!("fetch tokenizer.json failed: {err}"))
    })?;
    let weights = repo.get("model.safetensors").map_err(|err| {
        CalyxError::lens_unreachable(format!("fetch model.safetensors failed: {err}"))
    })?;
    Ok(CandleModelFiles {
        cache_dir: cache_dir.to_path_buf(),
        model_id: model_id.to_string(),
        config,
        tokenizer,
        weights,
        contract_paths: Vec::new(),
    })
}

pub(super) fn read_config(
    path: &Path,
    device_policy: CandleDevicePolicy,
    precision: CandlePrecision,
) -> Result<Config> {
    let bytes = std::fs::read(path).map_err(|err| {
        CalyxError::lens_unreachable(format!("read BERT config {} failed: {err}", path.display()))
    })?;
    let mut config: Config = serde_json::from_slice(&bytes)
        .map_err(|err| CalyxError::lens_unreachable(format!("parse BERT config failed: {err}")))?;
    stabilize_half_cuda_config(&mut config, device_policy, precision);
    Ok(config)
}

pub(super) fn stabilize_half_cuda_config(
    config: &mut Config,
    device_policy: CandleDevicePolicy,
    precision: CandlePrecision,
) {
    if matches!(device_policy, CandleDevicePolicy::CudaFailLoud { .. })
        && matches!(precision, CandlePrecision::F16 | CandlePrecision::BF16)
        && config.layer_norm_eps < HALF_CUDA_MIN_LAYER_NORM_EPS
    {
        config.layer_norm_eps = HALF_CUDA_MIN_LAYER_NORM_EPS;
    }
}

pub(super) fn needs_f32_finite_replay(
    device_policy: CandleDevicePolicy,
    precision: CandlePrecision,
) -> bool {
    matches!(device_policy, CandleDevicePolicy::CudaFailLoud { .. })
        && matches!(precision, CandlePrecision::F16 | CandlePrecision::BF16)
}

pub(super) fn read_tokenizer(path: &Path, max_tokens: usize) -> Result<Tokenizer> {
    let mut tokenizer = Tokenizer::from_file(path)
        .map_err(|err| CalyxError::lens_unreachable(format!("load tokenizer failed: {err}")))?;
    tokenizer
        .with_truncation(Some(TruncationParams {
            max_length: max_tokens,
            ..Default::default()
        }))
        .map_err(|err| CalyxError::lens_dim_mismatch(format!("set truncation failed: {err}")))?;
    Ok(tokenizer)
}

pub(super) fn read_model(
    weights: &Path,
    config: &Config,
    device_policy: CandleDevicePolicy,
    precision: CandlePrecision,
) -> Result<BertModel> {
    let device = candle_device(device_policy)?;
    let paths = [weights];
    let vb = unsafe { VarBuilder::from_mmaped_safetensors(&paths, precision.dtype(), &device) }
        .map_err(candle_error)?;
    BertModel::load(vb, config).map_err(candle_error)
}

pub(super) fn candle_device(policy: CandleDevicePolicy) -> Result<Device> {
    match policy {
        CandleDevicePolicy::CpuExplicit => Ok(Device::Cpu),
        CandleDevicePolicy::CudaFailLoud { ordinal } => candle_cuda_device(ordinal),
    }
}

#[cfg(feature = "candle-cuda")]
fn candle_cuda_device(ordinal: usize) -> Result<Device> {
    Device::new_cuda(ordinal)
        .map_err(|err| CalyxError::lens_unreachable(format!("candle CUDA init failed: {err}")))
}

#[cfg(not(feature = "candle-cuda"))]
fn candle_cuda_device(_ordinal: usize) -> Result<Device> {
    Err(CalyxError::lens_unreachable(
        "candle CUDA requested but calyx-registry was built without feature `candle-cuda`",
    ))
}

pub(super) fn candle_error(err: candle_core::Error) -> CalyxError {
    candle_error_message(format!("candle runtime failed: {err}"))
}

pub(super) fn candle_error_message(message: String) -> CalyxError {
    let lower = message.to_ascii_lowercase();
    if lower.contains("out of memory") || lower.contains("memoryallocation") {
        return CalyxError {
            code: "CALYX_VRAM_OOM",
            message,
            remediation: "free VRAM, reduce batch size, or evict lower-priority GPU lenses",
        };
    }
    CalyxError::lens_unreachable(message)
}

pub(super) fn ensure_file(label: &str, path: &Path) -> Result<()> {
    if path.is_file() {
        return Ok(());
    }
    Err(config_invalid(format!(
        "candle {label} file {} is missing",
        path.display()
    )))
}

pub(super) fn config_invalid(message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: "CALYX_LENS_CONFIG_INVALID",
        message: message.into(),
        remediation: "fix candle model/tokenizer/config or register a supported lens spec",
    }
}
