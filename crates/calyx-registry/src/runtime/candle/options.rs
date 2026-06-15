use std::path::PathBuf;

use calyx_core::Result;
use candle_core::DType;

use super::config_invalid;
use crate::frozen::NormPolicy;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CandleModelFiles {
    pub cache_dir: PathBuf,
    pub model_id: String,
    pub config: PathBuf,
    pub tokenizer: PathBuf,
    pub weights: PathBuf,
    pub contract_paths: Vec<PathBuf>,
}

impl CandleModelFiles {
    pub fn artifact_paths(&self) -> Vec<PathBuf> {
        if !self.contract_paths.is_empty() {
            return self.contract_paths.clone();
        }
        self.required_paths()
    }

    pub fn required_paths(&self) -> Vec<PathBuf> {
        vec![
            self.weights.clone(),
            self.tokenizer.clone(),
            self.config.clone(),
        ]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CandleDevicePolicy {
    CpuExplicit,
    CudaFailLoud { ordinal: usize },
}

impl CandleDevicePolicy {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CpuExplicit => "cpu_explicit,no_cuda",
            Self::CudaFailLoud { .. } => "cuda,error_on_failure,no_cpu_fallback",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CandlePrecision {
    F32,
    F16,
    BF16,
}

impl CandlePrecision {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::F32 => "f32",
            Self::F16 => "f16",
            Self::BF16 => "bf16",
        }
    }

    pub(crate) const fn dtype(self) -> DType {
        match self {
            Self::F32 => DType::F32,
            Self::F16 => DType::F16,
            Self::BF16 => DType::BF16,
        }
    }

    pub(crate) fn parse(raw: &str) -> Result<Self> {
        match raw {
            "f32" | "float32" => Ok(Self::F32),
            "f16" | "fp16" | "float16" => Ok(Self::F16),
            "bf16" | "bfloat16" => Ok(Self::BF16),
            other => Err(config_invalid(format!("unsupported candle dtype {other}"))),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CandlePoolingPolicy {
    Mean,
    Cls,
}

impl CandlePoolingPolicy {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Mean => "mean",
            Self::Cls => "cls",
        }
    }

    pub(crate) fn parse(raw: &str) -> Result<Self> {
        match raw {
            "mean" => Ok(Self::Mean),
            "cls" | "first_token" | "first-token" => Ok(Self::Cls),
            other => Err(config_invalid(format!(
                "unsupported candle pooling {other}"
            ))),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct CandleFileSpec {
    pub name: String,
    pub model_id: String,
    pub cache_dir: PathBuf,
    pub config: PathBuf,
    pub tokenizer: PathBuf,
    pub weights: PathBuf,
    pub max_tokens: usize,
    pub device_policy: CandleDevicePolicy,
    pub precision: CandlePrecision,
    pub pooling: CandlePoolingPolicy,
    pub norm_policy: NormPolicy,
    pub expected_dim: Option<u32>,
    pub expected_weights_sha256: Option<[u8; 32]>,
    pub contract_paths: Vec<PathBuf>,
}
