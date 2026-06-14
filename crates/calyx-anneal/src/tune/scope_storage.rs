mod types;
mod writer;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use calyx_core::{CalyxError, Result};
use calyx_forge::AutotuneCache;

use crate::{BanditPolicy, ConfigBandit, shape_key_hash};

pub use types::{
    DEFAULT_STORAGE_RECALL_TARGET, MAX_STORAGE_CANDIDATES, StorageConfig, StorageMetrics,
    StoragePromotionRecord, StorageShapeKey, StorageTuneDecision, candidate_storage_configs,
    decode_storage_config, encode_storage_config, storage_autotune_key, storage_shape_label,
    storage_win_check, validate_storage_config, validate_storage_metrics,
};
pub use writer::{
    NoopStorageBanditStore, NoopStoragePromotionWriter, StorageBanditPersistence,
    StoragePromotionWriter,
};

pub const CALYX_STORAGE_CACHE_WRITE_FAIL: &str = "CALYX_STORAGE_CACHE_WRITE_FAIL";
pub const CALYX_STORAGE_SCOPE_INVALID_CONFIG: &str = "CALYX_STORAGE_SCOPE_INVALID_CONFIG";

const NEXT_CHANGE_ID_START: u64 = 583_000;

pub struct StorageScopeTuner<W = NoopStoragePromotionWriter, B = NoopStorageBanditStore>
where
    W: StoragePromotionWriter,
    B: StorageBanditPersistence,
{
    pub bandits: HashMap<StorageShapeKey, ConfigBandit>,
    pub cache: Arc<Mutex<AutotuneCache>>,
    promotion_writer: W,
    bandit_store: B,
    pending_arms: HashMap<StorageShapeKey, usize>,
    incumbent_metrics: HashMap<StorageShapeKey, StorageMetrics>,
    next_change_id: u64,
    promotions: Vec<StoragePromotionRecord>,
}

impl StorageScopeTuner {
    pub fn new(cache: AutotuneCache) -> Self {
        Self::with_parts(cache, NoopStoragePromotionWriter, NoopStorageBanditStore)
    }
}

impl<W, B> StorageScopeTuner<W, B>
where
    W: StoragePromotionWriter,
    B: StorageBanditPersistence,
{
    pub fn with_parts(cache: AutotuneCache, promotion_writer: W, bandit_store: B) -> Self {
        Self {
            bandits: HashMap::new(),
            cache: Arc::new(Mutex::new(cache)),
            promotion_writer,
            bandit_store,
            pending_arms: HashMap::new(),
            incumbent_metrics: HashMap::new(),
            next_change_id: NEXT_CHANGE_ID_START,
            promotions: Vec::new(),
        }
    }

    pub fn on_observation(
        &mut self,
        key: StorageShapeKey,
        metrics: StorageMetrics,
    ) -> Result<StorageTuneDecision> {
        self.ensure_bandit(&key)?;
        let arm = self
            .pending_arms
            .remove(&key)
            .unwrap_or(self.bandits[&key].incumbent_idx);
        self.on_observation_for_arm(key, arm, metrics)
    }

    pub fn on_observation_for_arm(
        &mut self,
        key: StorageShapeKey,
        arm_idx: usize,
        metrics: StorageMetrics,
    ) -> Result<StorageTuneDecision> {
        validate_storage_metrics(&metrics)?;
        self.ensure_bandit(&key)?;
        let prior_idx = self.bandits[&key].incumbent_idx;
        let prior_config = self.config_for_arm(&key, prior_idx)?;
        let won = self.arm_won(&key, arm_idx, metrics)?;
        self.bandits
            .get_mut(&key)
            .expect("bandit ensured")
            .record_result(arm_idx, won)?;

        let new_idx = self.bandits[&key].incumbent_idx;
        let promoted = if new_idx != prior_idx {
            let new_config = self.config_for_arm(&key, new_idx)?;
            let before_metrics = self.incumbent_metrics.get(&key).copied().unwrap_or(metrics);
            self.record_incumbent_metrics(key.clone(), metrics);
            Some(self.promote(&key, prior_config, new_config, before_metrics, metrics)?)
        } else {
            if arm_idx == prior_idx {
                self.record_incumbent_metrics(key.clone(), metrics);
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

        Ok(StorageTuneDecision {
            evaluated_arm: arm_idx,
            won,
            incumbent: self.get_incumbent_config(&key)?,
            promoted,
            shadow_arm: Some(shadow_arm),
            shadow_candidate,
        })
    }

    pub fn install_candidates(
        &mut self,
        key: StorageShapeKey,
        configs: Vec<StorageConfig>,
    ) -> Result<()> {
        if configs.is_empty() || configs.len() > MAX_STORAGE_CANDIDATES {
            return Err(invalid_config(
                "Storage candidate set must contain 1..=8 configs",
            ));
        }
        let mut bandit = ConfigBandit::new(BanditPolicy::Thompson, types::seed_for_key(&key));
        for config in configs {
            validate_storage_config(&config)?;
            bandit.add_arm(encode_storage_config(&config)?);
        }
        let key_hash = shape_key_hash(&key.label());
        self.bandit_store.save_bandit(key_hash, &bandit)?;
        self.bandits.insert(key, bandit);
        Ok(())
    }

    pub fn get_incumbent_config(&self, key: &StorageShapeKey) -> Result<StorageConfig> {
        if let Some(bandit) = self.bandits.get(key)
            && !bandit.arms.is_empty()
        {
            return decode_storage_config(&bandit.incumbent()?.config);
        }
        let cache = self
            .cache
            .lock()
            .map_err(|_| invalid_config("autotune cache lock poisoned"))?;
        match cache.get(&storage_autotune_key(key, DEFAULT_STORAGE_RECALL_TARGET)) {
            Some(config) => StorageConfig::from_best_config(config),
            None => Ok(StorageConfig::default()),
        }
    }

    pub fn promotions(&self) -> &[StoragePromotionRecord] {
        &self.promotions
    }

    fn ensure_bandit(&mut self, key: &StorageShapeKey) -> Result<()> {
        if self.bandits.contains_key(key) {
            return Ok(());
        }
        let key_hash = shape_key_hash(&key.label());
        let bandit = match self.bandit_store.load_bandit(key_hash)? {
            Some(bandit) => bandit,
            None => {
                let mut bandit =
                    ConfigBandit::new(BanditPolicy::Thompson, types::seed_for_key(key));
                for config in candidate_storage_configs(key)? {
                    bandit.add_arm(encode_storage_config(&config)?);
                }
                self.bandit_store.save_bandit(key_hash, &bandit)?;
                bandit
            }
        };
        self.bandits.insert(key.clone(), bandit);
        Ok(())
    }

    fn arm_won(
        &self,
        key: &StorageShapeKey,
        arm_idx: usize,
        metrics: StorageMetrics,
    ) -> Result<bool> {
        let Some(bandit) = self.bandits.get(key) else {
            return Ok(false);
        };
        if arm_idx == bandit.incumbent_idx {
            return Ok(true);
        }
        let baseline = self.incumbent_metrics.get(key).copied().unwrap_or(metrics);
        Ok(storage_win_check(&baseline, &metrics))
    }

    fn promote(
        &mut self,
        key: &StorageShapeKey,
        old_config: StorageConfig,
        new_config: StorageConfig,
        metrics_before: StorageMetrics,
        metrics_after: StorageMetrics,
    ) -> Result<StoragePromotionRecord> {
        let change_id = crate::ChangeId(self.next_change_id);
        self.next_change_id = self.next_change_id.saturating_add(1);
        let old_bytes = encode_storage_config(&old_config)?;
        let new_bytes = encode_storage_config(&new_config)?;
        let record = StoragePromotionRecord {
            key: key.clone(),
            change_id,
            old_config,
            new_config,
            metrics_before,
            metrics_after,
            key_hash: shape_key_hash(&key.label()),
            old_config_hash: *blake3::hash(&old_bytes).as_bytes(),
            new_config_hash: *blake3::hash(&new_bytes).as_bytes(),
        };
        self.write_cache(&record)?;
        self.promotion_writer.write_autotune_promote(&record)?;
        self.promotions.push(record.clone());
        Ok(record)
    }

    fn write_cache(&self, record: &StoragePromotionRecord) -> Result<()> {
        let mut cache = self
            .cache
            .lock()
            .map_err(|_| invalid_config("autotune cache lock poisoned"))?;
        cache.insert(
            storage_autotune_key(&record.key, DEFAULT_STORAGE_RECALL_TARGET),
            record.new_config.to_best_config(&record.key),
        );
        cache.persist().map_err(cache_write_fail)
    }

    fn save_bandit(&self, key: &StorageShapeKey) -> Result<()> {
        self.bandit_store
            .save_bandit(shape_key_hash(&key.label()), &self.bandits[key])
    }

    fn config_for_arm(&self, key: &StorageShapeKey, arm_idx: usize) -> Result<StorageConfig> {
        let bandit = self
            .bandits
            .get(key)
            .ok_or_else(|| invalid_config("missing Storage bandit"))?;
        let arm = bandit
            .arms
            .get(arm_idx)
            .ok_or_else(|| invalid_config(format!("Storage arm {arm_idx} out of range")))?;
        decode_storage_config(&arm.config)
    }

    fn record_incumbent_metrics(&mut self, key: StorageShapeKey, metrics: StorageMetrics) {
        self.incumbent_metrics
            .entry(key)
            .and_modify(|current| current.keep_better_baseline(metrics))
            .or_insert(metrics);
    }
}

pub(super) fn invalid_config(message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: CALYX_STORAGE_SCOPE_INVALID_CONFIG,
        message: message.into(),
        remediation: "repair Storage scope autotune config, metrics, or cache metadata before promotion",
    }
}

fn cache_write_fail(error: calyx_forge::ForgeError) -> CalyxError {
    CalyxError {
        code: CALYX_STORAGE_CACHE_WRITE_FAIL,
        message: format!("Storage autotune cache write failed: {error}"),
        remediation: "repair the PH16 autotune cache path before persisting a Storage promotion",
    }
}
