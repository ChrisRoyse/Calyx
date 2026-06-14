use calyx_core::{Clock, Result};
use calyx_ledger::LedgerCfStore;
use serde_json::{Value, json};

use super::types::StoragePromotionRecord;
use crate::{
    AnnealLedger, AnnealLedgerAction, AnnealLedgerEntry, AnnealSubstrate, BanditStorage,
    ConfigBandit, ConfigBanditStore, MetricComparison, MetricSnapshot, TripwireMetric,
};

pub trait StoragePromotionWriter {
    fn write_autotune_promote(&mut self, event: &StoragePromotionRecord) -> Result<()>;
}

#[derive(Default)]
pub struct NoopStoragePromotionWriter;

impl StoragePromotionWriter for NoopStoragePromotionWriter {
    fn write_autotune_promote(&mut self, _event: &StoragePromotionRecord) -> Result<()> {
        Ok(())
    }
}

impl<T> StoragePromotionWriter for &mut T
where
    T: StoragePromotionWriter + ?Sized,
{
    fn write_autotune_promote(&mut self, event: &StoragePromotionRecord) -> Result<()> {
        (**self).write_autotune_promote(event)
    }
}

pub trait StorageBanditPersistence {
    fn load_bandit(&self, key_hash: [u8; 32]) -> Result<Option<ConfigBandit>>;
    fn save_bandit(&self, key_hash: [u8; 32], bandit: &ConfigBandit) -> Result<()>;
}

#[derive(Default)]
pub struct NoopStorageBanditStore;

impl StorageBanditPersistence for NoopStorageBanditStore {
    fn load_bandit(&self, _key_hash: [u8; 32]) -> Result<Option<ConfigBandit>> {
        Ok(None)
    }

    fn save_bandit(&self, _key_hash: [u8; 32], _bandit: &ConfigBandit) -> Result<()> {
        Ok(())
    }
}

impl<S> StorageBanditPersistence for ConfigBanditStore<S>
where
    S: BanditStorage,
{
    fn load_bandit(&self, key_hash: [u8; 32]) -> Result<Option<ConfigBandit>> {
        self.load(key_hash)
    }

    fn save_bandit(&self, key_hash: [u8; 32], bandit: &ConfigBandit) -> Result<()> {
        self.save(key_hash, bandit)
    }
}

impl<S, C> StoragePromotionWriter for AnnealLedger<S, C>
where
    S: LedgerCfStore,
    C: Clock,
{
    fn write_autotune_promote(&mut self, event: &StoragePromotionRecord) -> Result<()> {
        self.write(autotune_ledger_entry(event)).map(|_| ())
    }
}

impl<'a, R, L, C, P> StoragePromotionWriter for AnnealSubstrate<'a, R, L, C, P>
where
    R: crate::RollbackStorage,
    L: LedgerCfStore,
    C: Clock,
    P: crate::BudgetProbe,
{
    fn write_autotune_promote(&mut self, event: &StoragePromotionRecord) -> Result<()> {
        self.write_outcome_event_with_details(
            AnnealLedgerAction::AutotunePromote,
            event.change_id,
            storage_ledger_artifact_id(event),
            event.new_config_hash,
            promotion_description(event),
            Some(promotion_details(event)),
        )
    }
}

fn autotune_ledger_entry(event: &StoragePromotionRecord) -> AnnealLedgerEntry {
    AnnealLedgerEntry {
        action: AnnealLedgerAction::AutotunePromote,
        change_id: event.change_id,
        artifact_id: storage_ledger_artifact_id(event),
        prior_ptr_hash: event.old_config_hash,
        candidate_ptr_hash: event.new_config_hash,
        metrics: MetricSnapshot {
            evaluated_at: event.change_id.0,
            query_count: 1,
            metrics: vec![
                MetricComparison {
                    metric: TripwireMetric::SearchP99,
                    candidate_value: event.metrics_after.p99_read_ns as f64,
                    incumbent_value: event.metrics_before.p99_read_ns as f64,
                },
                MetricComparison {
                    metric: TripwireMetric::IngestP95,
                    candidate_value: event.metrics_after.write_amp_milli as f64,
                    incumbent_value: event.metrics_before.write_amp_milli as f64,
                },
            ],
        },
        ts: event.change_id.0,
        description: promotion_description(event),
        fault: None,
        proposal: None,
        details: Some(promotion_details(event)),
        prev_hash: None,
    }
}

fn promotion_details(event: &StoragePromotionRecord) -> Value {
    json!({
        "tag": "storage_autotune_promotion_v1",
        "scope": "storage",
        "shape_artifact": storage_ledger_artifact_id(event),
        "shape_hash_bytes": event.key_hash,
        "shape_bucketed": event.key.shape_bucketed,
        "old_config": event.old_config,
        "new_config": event.new_config,
        "metrics_before": event.metrics_before,
        "metrics_after": event.metrics_after,
    })
}

fn promotion_description(event: &StoragePromotionRecord) -> String {
    format!(
        "storage autotune promote {} p99 {} -> {} write_amp {} -> {} interval {} -> {} debt {} -> {} hot_hits {} -> {} cold_idle {} -> {} codebook {} -> {} prefetch {} -> {}",
        storage_ledger_artifact_id(event),
        event.metrics_before.p99_read_ns,
        event.metrics_after.p99_read_ns,
        event.metrics_before.write_amp_milli,
        event.metrics_after.write_amp_milli,
        event.old_config.compaction_interval_ms,
        event.new_config.compaction_interval_ms,
        event.old_config.debt_trigger_score_milli,
        event.new_config.debt_trigger_score_milli,
        event.old_config.hot_tier_min_hits,
        event.new_config.hot_tier_min_hits,
        event.old_config.cold_tier_idle_secs,
        event.new_config.cold_tier_idle_secs,
        event.old_config.codebook_refresh_secs,
        event.new_config.codebook_refresh_secs,
        event.old_config.prefetch_bytes,
        event.new_config.prefetch_bytes
    )
}

fn storage_ledger_artifact_id(event: &StoragePromotionRecord) -> String {
    let prefix = event.key_hash[..12]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("storage:{prefix}")
}
