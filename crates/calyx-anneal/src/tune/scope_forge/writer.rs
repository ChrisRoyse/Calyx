use calyx_core::{Clock, Result};
use calyx_ledger::LedgerCfStore;

use super::types::ForgePromotionRecord;
use crate::{
    AnnealLedger, AnnealLedgerAction, AnnealLedgerEntry, AnnealSubstrate, BanditStorage,
    ConfigBandit, ConfigBanditStore, MetricComparison, MetricSnapshot, TripwireMetric,
};

pub trait ForgePromotionWriter {
    fn write_autotune_promote(&mut self, event: &ForgePromotionRecord) -> Result<()>;
}

#[derive(Default)]
pub struct NoopForgePromotionWriter;

impl ForgePromotionWriter for NoopForgePromotionWriter {
    fn write_autotune_promote(&mut self, _event: &ForgePromotionRecord) -> Result<()> {
        Ok(())
    }
}

impl<T> ForgePromotionWriter for &mut T
where
    T: ForgePromotionWriter + ?Sized,
{
    fn write_autotune_promote(&mut self, event: &ForgePromotionRecord) -> Result<()> {
        (**self).write_autotune_promote(event)
    }
}

pub trait ForgeBanditPersistence {
    fn load_bandit(&self, key_hash: [u8; 32]) -> Result<Option<ConfigBandit>>;
    fn save_bandit(&self, key_hash: [u8; 32], bandit: &ConfigBandit) -> Result<()>;
}

#[derive(Default)]
pub struct NoopForgeBanditStore;

impl ForgeBanditPersistence for NoopForgeBanditStore {
    fn load_bandit(&self, _key_hash: [u8; 32]) -> Result<Option<ConfigBandit>> {
        Ok(None)
    }

    fn save_bandit(&self, _key_hash: [u8; 32], _bandit: &ConfigBandit) -> Result<()> {
        Ok(())
    }
}

impl<S> ForgeBanditPersistence for ConfigBanditStore<S>
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

impl<S, C> ForgePromotionWriter for AnnealLedger<S, C>
where
    S: LedgerCfStore,
    C: Clock,
{
    fn write_autotune_promote(&mut self, event: &ForgePromotionRecord) -> Result<()> {
        self.write(autotune_ledger_entry(event)).map(|_| ())
    }
}

impl<'a, R, L, C, P> ForgePromotionWriter for AnnealSubstrate<'a, R, L, C, P>
where
    R: crate::RollbackStorage,
    L: LedgerCfStore,
    C: Clock,
    P: crate::BudgetProbe,
{
    fn write_autotune_promote(&mut self, event: &ForgePromotionRecord) -> Result<()> {
        self.write_outcome_event(
            AnnealLedgerAction::AutotunePromote,
            event.change_id,
            event.key.label(),
            event.new_config_hash,
            promotion_description(event),
        )
    }
}

fn autotune_ledger_entry(event: &ForgePromotionRecord) -> AnnealLedgerEntry {
    AnnealLedgerEntry {
        action: AnnealLedgerAction::AutotunePromote,
        change_id: event.change_id,
        artifact_id: event.key.label(),
        prior_ptr_hash: event.old_config_hash,
        candidate_ptr_hash: event.new_config_hash,
        metrics: MetricSnapshot {
            evaluated_at: event.change_id.0,
            query_count: 1,
            metrics: vec![
                MetricComparison {
                    metric: TripwireMetric::SearchP99,
                    candidate_value: event.latency_after_ns as f64,
                    incumbent_value: event.latency_before_ns as f64,
                },
                MetricComparison {
                    metric: TripwireMetric::RecallAtK,
                    candidate_value: event.recall_after,
                    incumbent_value: event.recall_before,
                },
            ],
        },
        ts: event.change_id.0,
        description: promotion_description(event),
        fault: None,
        proposal: None,
        details: None,
        prev_hash: None,
    }
}

fn promotion_description(event: &ForgePromotionRecord) -> String {
    format!(
        "forge autotune promote {} latency {} -> {} recall {:.6} -> {:.6}",
        event.key.label(),
        event.latency_before_ns,
        event.latency_after_ns,
        event.recall_before,
        event.recall_after
    )
}
