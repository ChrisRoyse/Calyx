mod types;
mod writer;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use calyx_core::{CalyxError, Result};
use calyx_forge::AutotuneCache;

use crate::{BanditPolicy, ConfigBandit, shape_key_hash};

pub use types::{
    DEFAULT_FORGE_RECALL_TARGET, DType, ForgeConfig, ForgePromotionRecord, ForgeTuneDecision,
    MAX_BUCKETED_DIM, MAX_FORGE_CANDIDATES, ShapeKey, bucket_dim, bucket_shape, candidate_configs,
    decode_forge_config, encode_forge_config,
};
pub use writer::{
    ForgeBanditPersistence, ForgePromotionWriter, NoopForgeBanditStore, NoopForgePromotionWriter,
};

pub const CALYX_FORGE_CACHE_WRITE_FAIL: &str = "CALYX_FORGE_CACHE_WRITE_FAIL";
pub const CALYX_FORGE_SCOPE_INVALID_CONFIG: &str = "CALYX_FORGE_SCOPE_INVALID_CONFIG";

const NEXT_CHANGE_ID_START: u64 = 413_000;
const RECALL_EPSILON: f64 = 1e-12;

struct PromotionMetrics {
    latency_before_ns: u64,
    latency_after_ns: u64,
    recall_before: f64,
    recall_after: f64,
}

pub struct ForgeScopeTuner<W = NoopForgePromotionWriter, B = NoopForgeBanditStore>
where
    W: ForgePromotionWriter,
    B: ForgeBanditPersistence,
{
    pub bandits: HashMap<ShapeKey, ConfigBandit>,
    pub cache: Arc<Mutex<AutotuneCache>>,
    promotion_writer: W,
    bandit_store: B,
    pending_arms: HashMap<ShapeKey, usize>,
    incumbent_latency_ns: HashMap<ShapeKey, u64>,
    incumbent_recall: HashMap<ShapeKey, f64>,
    recall_target: f32,
    next_change_id: u64,
    promotions: Vec<ForgePromotionRecord>,
}

impl ForgeScopeTuner {
    pub fn new(cache: AutotuneCache) -> Self {
        Self::with_parts(cache, NoopForgePromotionWriter, NoopForgeBanditStore)
    }
}

impl<W, B> ForgeScopeTuner<W, B>
where
    W: ForgePromotionWriter,
    B: ForgeBanditPersistence,
{
    pub fn with_parts(cache: AutotuneCache, promotion_writer: W, bandit_store: B) -> Self {
        Self {
            bandits: HashMap::new(),
            cache: Arc::new(Mutex::new(cache)),
            promotion_writer,
            bandit_store,
            pending_arms: HashMap::new(),
            incumbent_latency_ns: HashMap::new(),
            incumbent_recall: HashMap::new(),
            recall_target: DEFAULT_FORGE_RECALL_TARGET,
            next_change_id: NEXT_CHANGE_ID_START,
            promotions: Vec::new(),
        }
    }

    pub fn on_op(
        &mut self,
        key: ShapeKey,
        elapsed_ns: u64,
        recall: f64,
    ) -> Result<ForgeTuneDecision> {
        self.ensure_bandit(&key)?;
        let arm = self
            .pending_arms
            .remove(&key)
            .unwrap_or(self.bandits[&key].incumbent_idx);
        self.on_op_for_arm(key, arm, elapsed_ns, recall)
    }

    pub fn on_op_for_arm(
        &mut self,
        key: ShapeKey,
        arm_idx: usize,
        elapsed_ns: u64,
        recall: f64,
    ) -> Result<ForgeTuneDecision> {
        self.ensure_bandit(&key)?;
        let prior_idx = self.bandits[&key].incumbent_idx;
        let prior_config = self.config_for_arm(&key, prior_idx)?;
        let won = self.arm_won(&key, arm_idx, elapsed_ns, recall);
        self.bandits
            .get_mut(&key)
            .expect("bandit ensured")
            .record_result(arm_idx, won)?;

        let new_idx = self.bandits[&key].incumbent_idx;
        let promoted = if new_idx != prior_idx {
            let new_config = self.config_for_arm(&key, new_idx)?;
            let latency_before_ns = self
                .incumbent_latency_ns
                .get(&key)
                .copied()
                .unwrap_or(elapsed_ns);
            let recall_before = self.incumbent_recall.get(&key).copied().unwrap_or(recall);
            self.incumbent_latency_ns.insert(key.clone(), elapsed_ns);
            self.incumbent_recall.insert(key.clone(), recall);
            Some(self.promote_with_metrics(
                &key,
                prior_config,
                new_config,
                PromotionMetrics {
                    latency_before_ns,
                    latency_after_ns: elapsed_ns,
                    recall_before,
                    recall_after: recall,
                },
            )?)
        } else {
            if arm_idx == prior_idx && recall.is_finite() {
                self.incumbent_latency_ns
                    .entry(key.clone())
                    .and_modify(|value| *value = (*value).min(elapsed_ns))
                    .or_insert(elapsed_ns);
                self.incumbent_recall.entry(key.clone()).or_insert(recall);
            }
            None
        };

        self.save_bandit(&key)?;
        let shadow_arm = self
            .bandits
            .get_mut(&key)
            .expect("bandit ensured")
            .select_arm()?;
        self.pending_arms.insert(key.clone(), shadow_arm);
        let shadow_candidate = (shadow_arm != new_idx)
            .then(|| self.config_for_arm(&key, shadow_arm))
            .transpose()?;

        Ok(ForgeTuneDecision {
            evaluated_arm: arm_idx,
            won,
            incumbent: self.get_incumbent(&key)?,
            promoted,
            shadow_arm: Some(shadow_arm),
            shadow_candidate,
        })
    }

    pub fn install_candidates(&mut self, key: ShapeKey, configs: Vec<ForgeConfig>) -> Result<()> {
        if configs.is_empty() || configs.len() > MAX_FORGE_CANDIDATES {
            return Err(invalid_config(
                "Forge candidate set must contain 1..=8 configs",
            ));
        }
        let mut bandit = ConfigBandit::new(BanditPolicy::Thompson, types::seed_for_key(&key));
        for config in configs {
            bandit.add_arm(encode_forge_config(&config)?);
        }
        let key_hash = shape_key_hash(&key.label());
        self.bandit_store.save_bandit(key_hash, &bandit)?;
        self.bandits.insert(key, bandit);
        Ok(())
    }

    pub fn get_incumbent(&self, key: &ShapeKey) -> Result<ForgeConfig> {
        if let Some(bandit) = self.bandits.get(key)
            && !bandit.arms.is_empty()
        {
            return decode_forge_config(&bandit.incumbent()?.config);
        }
        let cache = self
            .cache
            .lock()
            .map_err(|_| invalid_config("autotune cache lock poisoned"))?;
        Ok(cache
            .get(&key.autotune_key(self.recall_target))
            .map(|config| ForgeConfig::from_best_config(config, key.dtype))
            .unwrap_or_else(|| ForgeConfig::default_for(key)))
    }

    pub fn promotions(&self) -> &[ForgePromotionRecord] {
        &self.promotions
    }

    fn ensure_bandit(&mut self, key: &ShapeKey) -> Result<()> {
        if self.bandits.contains_key(key) {
            return Ok(());
        }
        let key_hash = shape_key_hash(&key.label());
        let bandit = match self.bandit_store.load_bandit(key_hash)? {
            Some(bandit) => bandit,
            None => {
                let mut bandit =
                    ConfigBandit::new(BanditPolicy::Thompson, types::seed_for_key(key));
                for config in candidate_configs(key)? {
                    bandit.add_arm(encode_forge_config(&config)?);
                }
                self.bandit_store.save_bandit(key_hash, &bandit)?;
                bandit
            }
        };
        self.bandits.insert(key.clone(), bandit);
        Ok(())
    }

    fn arm_won(&self, key: &ShapeKey, arm_idx: usize, elapsed_ns: u64, recall: f64) -> bool {
        if !recall.is_finite() {
            return false;
        }
        let Some(bandit) = self.bandits.get(key) else {
            return false;
        };
        if arm_idx == bandit.incumbent_idx {
            return true;
        }
        let baseline_latency = self
            .incumbent_latency_ns
            .get(key)
            .copied()
            .unwrap_or(elapsed_ns);
        let baseline_recall = self.incumbent_recall.get(key).copied().unwrap_or(recall);
        elapsed_ns < baseline_latency && recall + RECALL_EPSILON >= baseline_recall
    }

    fn promote_with_metrics(
        &mut self,
        key: &ShapeKey,
        old_config: ForgeConfig,
        new_config: ForgeConfig,
        metrics: PromotionMetrics,
    ) -> Result<ForgePromotionRecord> {
        let change_id = crate::ChangeId(self.next_change_id);
        self.next_change_id = self.next_change_id.saturating_add(1);
        let old_bytes = encode_forge_config(&old_config)?;
        let new_bytes = encode_forge_config(&new_config)?;
        let record = ForgePromotionRecord {
            key: key.clone(),
            change_id,
            old_config,
            new_config,
            latency_before_ns: metrics.latency_before_ns,
            latency_after_ns: metrics.latency_after_ns,
            recall_before: metrics.recall_before,
            recall_after: metrics.recall_after,
            key_hash: shape_key_hash(&key.label()),
            old_config_hash: *blake3::hash(&old_bytes).as_bytes(),
            new_config_hash: *blake3::hash(&new_bytes).as_bytes(),
        };
        self.write_cache(&record)?;
        self.promotion_writer.write_autotune_promote(&record)?;
        self.promotions.push(record.clone());
        Ok(record)
    }

    fn write_cache(&self, record: &ForgePromotionRecord) -> Result<()> {
        let mut cache = self
            .cache
            .lock()
            .map_err(|_| invalid_config("autotune cache lock poisoned"))?;
        cache.insert(
            record.key.autotune_key(self.recall_target),
            record.new_config.to_best_config(&record.key),
        );
        cache.persist().map_err(cache_write_fail)
    }

    fn save_bandit(&self, key: &ShapeKey) -> Result<()> {
        let key_hash = shape_key_hash(&key.label());
        self.bandit_store.save_bandit(key_hash, &self.bandits[key])
    }

    fn config_for_arm(&self, key: &ShapeKey, arm_idx: usize) -> Result<ForgeConfig> {
        let bandit = self
            .bandits
            .get(key)
            .ok_or_else(|| invalid_config("missing Forge bandit"))?;
        let arm = bandit
            .arms
            .get(arm_idx)
            .ok_or_else(|| invalid_config(format!("Forge arm {arm_idx} out of range")))?;
        decode_forge_config(&arm.config)
    }
}

pub(super) fn invalid_config(message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: CALYX_FORGE_SCOPE_INVALID_CONFIG,
        message: message.into(),
        remediation: "repair Forge scope autotune key/config metadata before promotion",
    }
}

fn cache_write_fail(error: calyx_forge::ForgeError) -> CalyxError {
    CalyxError {
        code: CALYX_FORGE_CACHE_WRITE_FAIL,
        message: format!("Forge autotune cache write failed: {error}"),
        remediation: "repair the PH16 autotune cache path before persisting a Forge promotion",
    }
}
