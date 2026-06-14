use std::collections::HashMap;
use std::path::PathBuf;

use calyx_aster::compaction::CompactionSchedulerOptions;
use calyx_core::Result;
use calyx_forge::{AutotuneKey, BackendKind, BestConfig};
use serde::{Deserialize, Serialize};

use super::invalid_config;
use crate::shape_key_hash;

pub const MAX_STORAGE_CANDIDATES: usize = 8;
pub const DEFAULT_STORAGE_RECALL_TARGET: f32 = 1.0;
const MAX_SHAPE_BUCKET: u32 = 1_048_576;
const MIN_INTERVAL_MS: u64 = 100;
const MAX_INTERVAL_MS: u64 = 600_000;
const MIN_DEBT_SCORE_MILLI: u64 = 100;
const MAX_DEBT_SCORE_MILLI: u64 = 10_000;
const MIN_WRITE_AMP_MILLI: u64 = 1_000;
const MAX_WRITE_AMP_MILLI: u64 = 10_000;
const MIN_COLD_IDLE_SECS: u64 = 60;
const MAX_COLD_IDLE_SECS: u64 = 31_536_000;
const MIN_CODEBOOK_REFRESH_SECS: u64 = 60;
const MAX_CODEBOOK_REFRESH_SECS: u64 = 604_800;
const MAX_PREFETCH_BYTES: u64 = 16 * 1024 * 1024;
const MAX_PER_MILLE: u64 = 1_000;

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct StorageShapeKey {
    pub vault_id: String,
    pub workload_id: String,
    pub shape_bucketed: Vec<u32>,
}

impl StorageShapeKey {
    pub fn new(vault_id: impl Into<String>, workload_id: impl Into<String>, shape: &[u32]) -> Self {
        Self {
            vault_id: vault_id.into(),
            workload_id: workload_id.into(),
            shape_bucketed: bucket_shape(shape),
        }
    }

    pub fn label(&self) -> String {
        storage_shape_label(&self.vault_id, &self.workload_id, &self.shape_bucketed)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct StorageConfig {
    pub compaction_interval_ms: u64,
    pub debt_trigger_score_milli: u64,
    pub max_write_amp_milli: u64,
    pub hot_tier_min_hits: u64,
    pub cold_tier_idle_secs: u64,
    pub codebook_refresh_secs: u64,
    pub prefetch_bytes: u64,
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            compaction_interval_ms: 10_000,
            debt_trigger_score_milli: 1_000,
            max_write_amp_milli: 2_000,
            hot_tier_min_hits: 8,
            cold_tier_idle_secs: 86_400,
            codebook_refresh_secs: 3_600,
            prefetch_bytes: 64 * 1024,
        }
    }
}

impl StorageConfig {
    pub fn to_best_config(&self, key: &StorageShapeKey) -> BestConfig {
        BestConfig {
            backend: BackendKind::Cpu,
            tile_m: self.compaction_interval_ms as usize,
            tile_n: self.debt_trigger_score_milli as usize,
            tile_k: self.max_write_amp_milli as usize,
            extra: HashMap::from([
                ("scope".to_string(), "storage".to_string()),
                ("source".to_string(), "anneal-storage-scope".to_string()),
                ("shape_key".to_string(), key.label()),
                (
                    "shape_key_hash".to_string(),
                    hex_bytes(&shape_key_hash(&key.label())),
                ),
                ("vault_id".to_string(), key.vault_id.clone()),
                ("workload_id".to_string(), key.workload_id.clone()),
                (
                    "compaction_interval_ms".to_string(),
                    self.compaction_interval_ms.to_string(),
                ),
                (
                    "debt_trigger_score_milli".to_string(),
                    self.debt_trigger_score_milli.to_string(),
                ),
                (
                    "max_write_amp_milli".to_string(),
                    self.max_write_amp_milli.to_string(),
                ),
                (
                    "hot_tier_min_hits".to_string(),
                    self.hot_tier_min_hits.to_string(),
                ),
                (
                    "cold_tier_idle_secs".to_string(),
                    self.cold_tier_idle_secs.to_string(),
                ),
                (
                    "codebook_refresh_secs".to_string(),
                    self.codebook_refresh_secs.to_string(),
                ),
                (
                    "prefetch_bytes".to_string(),
                    self.prefetch_bytes.to_string(),
                ),
            ]),
        }
    }

    pub fn from_best_config(config: &BestConfig) -> Result<Self> {
        let parsed = Self {
            compaction_interval_ms: parse_u64(config, "compaction_interval_ms")
                .unwrap_or(config.tile_m as u64),
            debt_trigger_score_milli: parse_u64(config, "debt_trigger_score_milli")
                .unwrap_or(config.tile_n as u64),
            max_write_amp_milli: parse_u64(config, "max_write_amp_milli")
                .unwrap_or(config.tile_k as u64),
            hot_tier_min_hits: parse_u64(config, "hot_tier_min_hits").unwrap_or(8),
            cold_tier_idle_secs: parse_u64(config, "cold_tier_idle_secs").unwrap_or(86_400),
            codebook_refresh_secs: parse_u64(config, "codebook_refresh_secs").unwrap_or(3_600),
            prefetch_bytes: parse_u64(config, "prefetch_bytes").unwrap_or(64 * 1024),
        };
        validate_storage_config(&parsed)?;
        Ok(parsed)
    }

    pub fn to_scheduler_options(
        &self,
        output_root: impl Into<PathBuf>,
    ) -> Result<CompactionSchedulerOptions> {
        validate_storage_config(self)?;
        Ok(CompactionSchedulerOptions {
            interval_ms: self.compaction_interval_ms,
            debt_trigger_score_milli: self.debt_trigger_score_milli,
            max_write_amp_milli: self.max_write_amp_milli,
            output_root: output_root.into(),
            ..CompactionSchedulerOptions::default()
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct StorageMetrics {
    pub p99_read_ns: u64,
    pub write_amp_milli: u64,
    pub cache_miss_milli: u64,
    pub tier_hot_hit_milli: u64,
    pub codebook_staleness_secs: u64,
    pub prefetch_hit_milli: u64,
}

impl StorageMetrics {
    pub fn keep_better_baseline(&mut self, observed: Self) {
        self.p99_read_ns = self.p99_read_ns.min(observed.p99_read_ns);
        self.write_amp_milli = self.write_amp_milli.min(observed.write_amp_milli);
        self.cache_miss_milli = self.cache_miss_milli.min(observed.cache_miss_milli);
        self.tier_hot_hit_milli = self.tier_hot_hit_milli.max(observed.tier_hot_hit_milli);
        self.codebook_staleness_secs = self
            .codebook_staleness_secs
            .min(observed.codebook_staleness_secs);
        self.prefetch_hit_milli = self.prefetch_hit_milli.max(observed.prefetch_hit_milli);
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StoragePromotionRecord {
    pub key: StorageShapeKey,
    pub change_id: crate::ChangeId,
    pub old_config: StorageConfig,
    pub new_config: StorageConfig,
    pub metrics_before: StorageMetrics,
    pub metrics_after: StorageMetrics,
    pub key_hash: [u8; 32],
    pub old_config_hash: [u8; 32],
    pub new_config_hash: [u8; 32],
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StorageTuneDecision {
    pub evaluated_arm: usize,
    pub won: bool,
    pub incumbent: StorageConfig,
    pub promoted: Option<StoragePromotionRecord>,
    pub shadow_arm: Option<usize>,
    pub shadow_candidate: Option<StorageConfig>,
}

pub fn candidate_storage_configs(key: &StorageShapeKey) -> Result<Vec<StorageConfig>> {
    let base = StorageConfig::default();
    let read_bucket = key.shape_bucketed.first().copied().unwrap_or(1);
    let prefetch_large = if read_bucket >= 1_024 {
        256 * 1024
    } else {
        128 * 1024
    };
    let candidates = [
        base.clone(),
        StorageConfig {
            compaction_interval_ms: 5_000,
            debt_trigger_score_milli: 750,
            ..base.clone()
        },
        StorageConfig {
            compaction_interval_ms: 30_000,
            debt_trigger_score_milli: 2_000,
            ..base.clone()
        },
        StorageConfig {
            hot_tier_min_hits: 4,
            cold_tier_idle_secs: 3_600,
            ..base.clone()
        },
        StorageConfig {
            hot_tier_min_hits: 16,
            cold_tier_idle_secs: 7 * 86_400,
            ..base.clone()
        },
        StorageConfig {
            codebook_refresh_secs: 900,
            ..base.clone()
        },
        StorageConfig {
            prefetch_bytes: prefetch_large,
            ..base.clone()
        },
        StorageConfig {
            prefetch_bytes: 0,
            ..base
        },
    ];
    let mut configs = Vec::with_capacity(MAX_STORAGE_CANDIDATES);
    for config in candidates {
        validate_storage_config(&config)?;
        push_unique(&mut configs, config);
    }
    Ok(configs)
}

pub fn storage_win_check(before: &StorageMetrics, after: &StorageMetrics) -> bool {
    validate_storage_metrics(before).is_ok()
        && validate_storage_metrics(after).is_ok()
        && after.p99_read_ns < before.p99_read_ns
        && after.write_amp_milli <= before.write_amp_milli
        && after.cache_miss_milli <= before.cache_miss_milli
        && after.tier_hot_hit_milli >= before.tier_hot_hit_milli
        && after.codebook_staleness_secs <= before.codebook_staleness_secs
        && after.prefetch_hit_milli >= before.prefetch_hit_milli
}

pub fn validate_storage_config(config: &StorageConfig) -> Result<()> {
    require_range(
        "compaction_interval_ms",
        config.compaction_interval_ms,
        MIN_INTERVAL_MS,
        MAX_INTERVAL_MS,
    )?;
    require_range(
        "debt_trigger_score_milli",
        config.debt_trigger_score_milli,
        MIN_DEBT_SCORE_MILLI,
        MAX_DEBT_SCORE_MILLI,
    )?;
    require_range(
        "max_write_amp_milli",
        config.max_write_amp_milli,
        MIN_WRITE_AMP_MILLI,
        MAX_WRITE_AMP_MILLI,
    )?;
    if config.hot_tier_min_hits == 0 {
        return Err(invalid_config("hot_tier_min_hits must be nonzero"));
    }
    require_range(
        "cold_tier_idle_secs",
        config.cold_tier_idle_secs,
        MIN_COLD_IDLE_SECS,
        MAX_COLD_IDLE_SECS,
    )?;
    require_range(
        "codebook_refresh_secs",
        config.codebook_refresh_secs,
        MIN_CODEBOOK_REFRESH_SECS,
        MAX_CODEBOOK_REFRESH_SECS,
    )?;
    if config.prefetch_bytes > MAX_PREFETCH_BYTES {
        return Err(invalid_config(format!(
            "prefetch_bytes {} exceeds {}",
            config.prefetch_bytes, MAX_PREFETCH_BYTES
        )));
    }
    if config.prefetch_bytes != 0 && !config.prefetch_bytes.is_multiple_of(4096) {
        return Err(invalid_config(
            "prefetch_bytes must be zero or a 4096-byte multiple",
        ));
    }
    Ok(())
}

pub fn validate_storage_metrics(metrics: &StorageMetrics) -> Result<()> {
    if metrics.p99_read_ns == 0 {
        return Err(invalid_config("storage p99_read_ns must be nonzero"));
    }
    if metrics.write_amp_milli == 0 {
        return Err(invalid_config("storage write_amp_milli must be nonzero"));
    }
    if metrics.cache_miss_milli > MAX_PER_MILLE
        || metrics.tier_hot_hit_milli > MAX_PER_MILLE
        || metrics.prefetch_hit_milli > MAX_PER_MILLE
    {
        return Err(invalid_config(
            "storage per-mille metrics must be between 0 and 1000",
        ));
    }
    Ok(())
}

pub fn encode_storage_config(config: &StorageConfig) -> Result<Vec<u8>> {
    validate_storage_config(config)?;
    let mut bytes = Vec::new();
    ciborium::ser::into_writer(config, &mut bytes)
        .map_err(|error| invalid_config(error.to_string()))?;
    Ok(bytes)
}

pub fn decode_storage_config(bytes: &[u8]) -> Result<StorageConfig> {
    let config: StorageConfig =
        ciborium::de::from_reader(bytes).map_err(|error| invalid_config(error.to_string()))?;
    validate_storage_config(&config)?;
    Ok(config)
}

pub fn storage_autotune_key(key: &StorageShapeKey, recall_target: f32) -> AutotuneKey {
    AutotuneKey {
        op: "storage".to_string(),
        shape: key.shape_bucketed.iter().map(|dim| *dim as usize).collect(),
        dtype: "aster-storage".to_string(),
        device: format!("{}:{}", key.vault_id, key.workload_id),
        recall_tgt: recall_target,
    }
}

pub fn storage_shape_label(vault_id: &str, workload_id: &str, shape: &[u32]) -> String {
    let shape = shape
        .iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join("x");
    format!("storage:{vault_id}:{workload_id}:{shape}")
}

pub(super) fn seed_for_key(key: &StorageShapeKey) -> u64 {
    let hash = shape_key_hash(&key.label());
    u64::from_le_bytes(hash[0..8].try_into().expect("hash slice has 8 bytes"))
}

fn bucket_shape(shape: &[u32]) -> Vec<u32> {
    shape.iter().map(|dim| bucket_dim(*dim)).collect()
}

fn bucket_dim(dim: u32) -> u32 {
    match dim {
        0 | 1 => 1,
        value if value >= MAX_SHAPE_BUCKET => MAX_SHAPE_BUCKET,
        value => value.next_power_of_two(),
    }
}

fn push_unique(configs: &mut Vec<StorageConfig>, config: StorageConfig) {
    if !configs.contains(&config) {
        configs.push(config);
    }
}

fn parse_u64(config: &BestConfig, field: &str) -> Option<u64> {
    config.extra.get(field).and_then(|value| value.parse().ok())
}

fn require_range(field: &str, value: u64, min: u64, max: u64) -> Result<()> {
    if value < min || value > max {
        return Err(invalid_config(format!(
            "{field} {value} outside {min}..={max}"
        )));
    }
    Ok(())
}

fn hex_bytes(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
