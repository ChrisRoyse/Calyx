use calyx_core::{Clock, Result};
use calyx_ledger::LedgerCfStore;

use super::LoomPromotionRecord;
use super::types::loom_plan_label;
use crate::{
    AnnealLedger, AnnealLedgerAction, AnnealLedgerEntry, AnnealSubstrate, BanditStorage,
    ConfigBandit, ConfigBanditStore, MetricComparison, MetricSnapshot, TripwireMetric,
};

pub trait LoomPromotionWriter {
    fn write_autotune_promote(&mut self, event: &LoomPromotionRecord) -> Result<()>;
}

#[derive(Default)]
pub struct NoopLoomPromotionWriter;

impl LoomPromotionWriter for NoopLoomPromotionWriter {
    fn write_autotune_promote(&mut self, _event: &LoomPromotionRecord) -> Result<()> {
        Ok(())
    }
}

impl<T> LoomPromotionWriter for &mut T
where
    T: LoomPromotionWriter + ?Sized,
{
    fn write_autotune_promote(&mut self, event: &LoomPromotionRecord) -> Result<()> {
        (**self).write_autotune_promote(event)
    }
}

pub trait LoomBanditPersistence {
    fn load_bandit(&self, key_hash: [u8; 32]) -> Result<Option<ConfigBandit>>;
    fn save_bandit(&self, key_hash: [u8; 32], bandit: &ConfigBandit) -> Result<()>;
}

#[derive(Default)]
pub struct NoopLoomBanditStore;

impl LoomBanditPersistence for NoopLoomBanditStore {
    fn load_bandit(&self, _key_hash: [u8; 32]) -> Result<Option<ConfigBandit>> {
        Ok(None)
    }

    fn save_bandit(&self, _key_hash: [u8; 32], _bandit: &ConfigBandit) -> Result<()> {
        Ok(())
    }
}

impl<S> LoomBanditPersistence for ConfigBanditStore<S>
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

impl<S, C> LoomPromotionWriter for AnnealLedger<S, C>
where
    S: LedgerCfStore,
    C: Clock,
{
    fn write_autotune_promote(&mut self, event: &LoomPromotionRecord) -> Result<()> {
        self.write(autotune_ledger_entry(event)).map(|_| ())
    }
}

impl<'a, R, L, C, P> LoomPromotionWriter for AnnealSubstrate<'a, R, L, C, P>
where
    R: crate::RollbackStorage,
    L: LedgerCfStore,
    C: Clock,
    P: crate::BudgetProbe,
{
    fn write_autotune_promote(&mut self, event: &LoomPromotionRecord) -> Result<()> {
        self.write_outcome_event(
            AnnealLedgerAction::AutotunePromote,
            event.change_id,
            loom_plan_label(),
            event.new_plan_hash,
            promotion_description(event),
        )
    }
}

fn autotune_ledger_entry(event: &LoomPromotionRecord) -> AnnealLedgerEntry {
    AnnealLedgerEntry {
        action: AnnealLedgerAction::AutotunePromote,
        change_id: event.change_id,
        artifact_id: loom_plan_label(),
        prior_ptr_hash: event.old_plan_hash,
        candidate_ptr_hash: event.new_plan_hash,
        metrics: MetricSnapshot {
            evaluated_at: event.change_id.0,
            query_count: 1,
            metrics: vec![MetricComparison {
                metric: TripwireMetric::SearchP99,
                candidate_value: event.latency_after_ns as f64,
                incumbent_value: event.latency_before_ns as f64,
            }],
        },
        ts: event.change_id.0,
        description: promotion_description(event),
        fault: None,
        proposal: None,
        details: None,
        prev_hash: None,
    }
}

fn promotion_description(event: &LoomPromotionRecord) -> String {
    format!(
        "loom autotune promote eager_pairs {} -> {} concat_indexes {} -> {} latency {} -> {} bits {:.6} -> {:.6}",
        event.old_plan.eager_pairs.len(),
        event.new_plan.eager_pairs.len(),
        event.old_plan.indexed_concat_keys.len(),
        event.new_plan.indexed_concat_keys.len(),
        event.latency_before_ns,
        event.latency_after_ns,
        event.bits_before,
        event.bits_after
    )
}
