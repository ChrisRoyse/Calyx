use std::path::{Path, PathBuf};

use calyx_core::{CalyxError, Result, SlotVector};
use candle_core::Device;
use candle_nn::VarBuilder;
use fastembed::{Qwen3Config, Qwen3Model, Qwen3TextEmbedding};
use tokenizers::{PaddingDirection, PaddingParams, PaddingStrategy, Tokenizer, TruncationParams};

use super::{DEFAULT_QWEN3_MODEL, config_invalid, qwen3_error};
use crate::runtime::candle::{CandleDevicePolicy, CandlePrecision};
use crate::runtime::common::normalize_unit;

pub fn read_config(path: &Path) -> Result<Qwen3Config> {
    let bytes = std::fs::read(path).map_err(|err| {
        CalyxError::lens_unreachable(format!(
            "read Qwen3 config {} failed: {err}",
            path.display()
        ))
    })?;
    serde_json::from_slice(&bytes)
        .map_err(|err| config_invalid(format!("parse Qwen3 config failed: {err}")))
}

pub fn read_tokenizer(path: &Path, max_tokens: usize) -> Result<Tokenizer> {
    let mut tokenizer = Tokenizer::from_file(path).map_err(|err| {
        CalyxError::lens_unreachable(format!("load Qwen3 tokenizer failed: {err}"))
    })?;
    tokenizer.with_padding(Some(PaddingParams {
        strategy: PaddingStrategy::BatchLongest,
        direction: PaddingDirection::Left,
        ..Default::default()
    }));
    tokenizer
        .with_truncation(Some(TruncationParams {
            max_length: max_tokens,
            ..Default::default()
        }))
        .map_err(|err| CalyxError::lens_dim_mismatch(format!("set truncation failed: {err}")))?;
    Ok(tokenizer)
}

pub fn read_model(
    weights: &[PathBuf],
    config: Qwen3Config,
    tokenizer: Tokenizer,
    device_policy: CandleDevicePolicy,
    precision: CandlePrecision,
) -> Result<Qwen3TextEmbedding> {
    let device = qwen3_device(device_policy)?;
    let vb = unsafe { VarBuilder::from_mmaped_safetensors(weights, precision.dtype(), &device) }
        .map_err(qwen3_error)?;
    let model = Qwen3Model::new(config, vb).map_err(qwen3_error)?;
    Ok(Qwen3TextEmbedding::new(model, tokenizer))
}

pub fn dense_batch(dim: u32, rows: Vec<Vec<f32>>, expected: usize) -> Result<Vec<SlotVector>> {
    if rows.len() != expected {
        return Err(CalyxError::lens_dim_mismatch(format!(
            "Qwen3 returned {} vectors for {expected} inputs",
            rows.len()
        )));
    }
    rows.into_iter()
        .map(|mut data| {
            if data.len() != dim as usize {
                return Err(CalyxError::lens_dim_mismatch(format!(
                    "Qwen3 dim {} != expected {dim}",
                    data.len()
                )));
            }
            normalize_unit(&mut data)?;
            Ok(SlotVector::Dense { dim, data })
        })
        .collect()
}

pub fn qwen3_model_id(raw: &str) -> Result<String> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "qwen/qwen3-embedding-0.6b" | "qwen3-embedding-0.6b" | "qwen3-0.6b" => {
            Ok(DEFAULT_QWEN3_MODEL.to_string())
        }
        other => Err(CalyxError::lens_unreachable(format!(
            "unsupported fastembed-qwen3 model {other}; expected {DEFAULT_QWEN3_MODEL}"
        ))),
    }
}

fn qwen3_device(policy: CandleDevicePolicy) -> Result<Device> {
    match policy {
        CandleDevicePolicy::CpuExplicit => Ok(Device::Cpu),
        CandleDevicePolicy::CudaFailLoud { ordinal } => qwen3_cuda_device(ordinal),
    }
}

#[cfg(feature = "candle-cuda")]
fn qwen3_cuda_device(ordinal: usize) -> Result<Device> {
    Device::new_cuda(ordinal)
        .map_err(|err| CalyxError::lens_unreachable(format!("Qwen3 CUDA init failed: {err}")))
}

#[cfg(not(feature = "candle-cuda"))]
fn qwen3_cuda_device(_ordinal: usize) -> Result<Device> {
    Err(CalyxError::lens_unreachable(
        "Qwen3 CUDA requested but calyx-registry was built without feature `candle-cuda`",
    ))
}
