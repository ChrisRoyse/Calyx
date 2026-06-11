use std::collections::HashMap;
use std::fmt;

use calyx_core::Result;
use calyx_forge::{AutotuneKey, BackendKind, BestConfig};
use serde::{Deserialize, Serialize};

use super::invalid_config;
use crate::shape_key_hash;

pub const MAX_FORGE_CANDIDATES: usize = 8;
pub const MAX_BUCKETED_DIM: u32 = 65_536;
pub const DEFAULT_FORGE_RECALL_TARGET: f32 = 0.99;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DType {
    Fp32,
    Fp16,
    Bf16,
    Fp8,
}

impl fmt::Display for DType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Fp32 => "fp32",
            Self::Fp16 => "fp16",
            Self::Bf16 => "bf16",
            Self::Fp8 => "fp8",
        })
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct ShapeKey {
    pub op_id: String,
    pub shape_bucketed: Vec<u32>,
    pub dtype: DType,
    pub device_id: String,
}

impl ShapeKey {
    pub fn new(
        op_id: impl Into<String>,
        shape: &[u32],
        dtype: DType,
        device_id: impl Into<String>,
    ) -> Self {
        Self {
            op_id: op_id.into(),
            shape_bucketed: bucket_shape(shape),
            dtype,
            device_id: device_id.into(),
        }
    }

    pub fn label(&self) -> String {
        let shape = self
            .shape_bucketed
            .iter()
            .map(u32::to_string)
            .collect::<Vec<_>>()
            .join("x");
        format!(
            "forge:{}:{}:{}:{}",
            self.op_id, shape, self.dtype, self.device_id
        )
    }

    pub fn autotune_key(&self, recall_target: f32) -> AutotuneKey {
        AutotuneKey {
            op: self.op_id.clone(),
            shape: self
                .shape_bucketed
                .iter()
                .map(|dim| *dim as usize)
                .collect(),
            dtype: self.dtype.to_string(),
            device: self.device_id.clone(),
            recall_tgt: recall_target,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ForgeConfig {
    pub tile_m: u32,
    pub tile_n: u32,
    pub tile_k: u32,
    pub dtype: DType,
    pub batch_size: u32,
}

impl ForgeConfig {
    pub fn default_for(key: &ShapeKey) -> Self {
        Self {
            tile_m: 64,
            tile_n: 64,
            tile_k: 32,
            dtype: key.dtype,
            batch_size: 1,
        }
    }

    pub fn to_best_config(&self, key: &ShapeKey) -> BestConfig {
        BestConfig {
            backend: backend_for_device(&key.device_id),
            tile_m: self.tile_m as usize,
            tile_n: self.tile_n as usize,
            tile_k: self.tile_k as usize,
            extra: HashMap::from([
                ("op".to_string(), key.op_id.clone()),
                ("dtype".to_string(), self.dtype.to_string()),
                ("batch_size".to_string(), self.batch_size.to_string()),
                ("device".to_string(), key.device_id.clone()),
                ("source".to_string(), "anneal-forge-scope".to_string()),
            ]),
        }
    }

    pub fn from_best_config(config: &BestConfig, fallback_dtype: DType) -> Self {
        Self {
            tile_m: config.tile_m as u32,
            tile_n: config.tile_n as u32,
            tile_k: config.tile_k as u32,
            dtype: config
                .extra
                .get("dtype")
                .and_then(|value| parse_dtype(value))
                .unwrap_or(fallback_dtype),
            batch_size: config
                .extra
                .get("batch_size")
                .and_then(|value| value.parse::<u32>().ok())
                .unwrap_or(1),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ForgePromotionRecord {
    pub key: ShapeKey,
    pub change_id: crate::ChangeId,
    pub old_config: ForgeConfig,
    pub new_config: ForgeConfig,
    pub latency_before_ns: u64,
    pub latency_after_ns: u64,
    pub recall_before: f64,
    pub recall_after: f64,
    pub key_hash: [u8; 32],
    pub old_config_hash: [u8; 32],
    pub new_config_hash: [u8; 32],
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ForgeTuneDecision {
    pub evaluated_arm: usize,
    pub won: bool,
    pub incumbent: ForgeConfig,
    pub promoted: Option<ForgePromotionRecord>,
    pub shadow_arm: Option<usize>,
    pub shadow_candidate: Option<ForgeConfig>,
}

pub fn candidate_configs(key: &ShapeKey) -> Result<Vec<ForgeConfig>> {
    let max_dim = key.shape_bucketed.iter().copied().max().unwrap_or(64);
    let base = ForgeConfig::default_for(key);
    let wide = if max_dim >= 1024 { 128 } else { 64 };
    let mut configs = Vec::with_capacity(MAX_FORGE_CANDIDATES);
    push_unique(&mut configs, base.clone());
    push_unique(
        &mut configs,
        ForgeConfig {
            tile_m: wide,
            tile_n: 64,
            ..base.clone()
        },
    );
    push_unique(
        &mut configs,
        ForgeConfig {
            tile_m: 64,
            tile_n: wide,
            ..base.clone()
        },
    );
    push_unique(
        &mut configs,
        ForgeConfig {
            tile_m: wide,
            tile_n: wide,
            ..base.clone()
        },
    );
    push_unique(
        &mut configs,
        ForgeConfig {
            tile_m: wide,
            tile_n: wide,
            tile_k: 64,
            ..base.clone()
        },
    );
    push_unique(
        &mut configs,
        ForgeConfig {
            batch_size: 2,
            ..base.clone()
        },
    );
    if key.device_id.contains("cuda") && key.dtype != DType::Fp32 {
        push_unique(
            &mut configs,
            ForgeConfig {
                dtype: DType::Bf16,
                ..base.clone()
            },
        );
        push_unique(
            &mut configs,
            ForgeConfig {
                dtype: DType::Fp8,
                ..base
            },
        );
    }
    configs.truncate(MAX_FORGE_CANDIDATES);
    Ok(configs)
}

pub fn encode_forge_config(config: &ForgeConfig) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    ciborium::ser::into_writer(config, &mut bytes)
        .map_err(|error| invalid_config(error.to_string()))?;
    Ok(bytes)
}

pub fn decode_forge_config(bytes: &[u8]) -> Result<ForgeConfig> {
    ciborium::de::from_reader(bytes).map_err(|error| invalid_config(error.to_string()))
}

pub fn bucket_shape(shape: &[u32]) -> Vec<u32> {
    shape.iter().map(|dim| bucket_dim(*dim)).collect()
}

pub fn bucket_dim(dim: u32) -> u32 {
    match dim {
        0 | 1 => 1,
        value if value >= MAX_BUCKETED_DIM => MAX_BUCKETED_DIM,
        value => value.next_power_of_two(),
    }
}

pub(super) fn seed_for_key(key: &ShapeKey) -> u64 {
    let hash = shape_key_hash(&key.label());
    u64::from_le_bytes(hash[0..8].try_into().expect("hash slice has 8 bytes"))
}

fn push_unique(configs: &mut Vec<ForgeConfig>, config: ForgeConfig) {
    if !configs.contains(&config) {
        configs.push(config);
    }
}

fn backend_for_device(device: &str) -> BackendKind {
    if device.contains("cuda") {
        BackendKind::Cuda
    } else {
        BackendKind::Cpu
    }
}

fn parse_dtype(value: &str) -> Option<DType> {
    match value {
        "fp32" => Some(DType::Fp32),
        "fp16" => Some(DType::Fp16),
        "bf16" => Some(DType::Bf16),
        "fp8" => Some(DType::Fp8),
        _ => None,
    }
}
