use calyx_core::{Clock, Result};
use calyx_ledger::LedgerCfStore;

use crate::{
    AnnealLedger, AnnealLedgerEntry, BudgetEnforcer, BudgetHandle, BudgetProbe, MetricComparison,
    MetricSnapshot, TripwireMetric,
};

use super::errors::budget_exhausted;
use super::types::ABLedgerEvent;

const DEFAULT_SHADOW_CPU_WEIGHT: f64 = 0.01;
const DEFAULT_SHADOW_VRAM_BYTES: u64 = 0;

pub trait ABTrialBudget {
    fn acquire_shadow(&self) -> Result<BudgetHandle>;
}

#[derive(Clone, Copy, Debug)]
pub struct NoopABBudget {
    pub ticks: usize,
}

impl Default for NoopABBudget {
    fn default() -> Self {
        Self { ticks: 1 }
    }
}

impl ABTrialBudget for NoopABBudget {
    fn acquire_shadow(&self) -> Result<BudgetHandle> {
        if self.ticks == 0 {
            return Err(budget_exhausted("scripted A/B shadow budget exhausted"));
        }
        Ok(BudgetHandle::new(self.ticks))
    }
}

impl<P> ABTrialBudget for BudgetEnforcer<'_, P>
where
    P: BudgetProbe,
{
    fn acquire_shadow(&self) -> Result<BudgetHandle> {
        self.acquire(DEFAULT_SHADOW_CPU_WEIGHT, DEFAULT_SHADOW_VRAM_BYTES)
    }
}

pub trait ABLedgerWriter {
    fn write_ab_event(&mut self, event: &ABLedgerEvent) -> Result<()>;
}

#[derive(Default)]
pub struct NoopABLedgerWriter;

impl ABLedgerWriter for NoopABLedgerWriter {
    fn write_ab_event(&mut self, _event: &ABLedgerEvent) -> Result<()> {
        Ok(())
    }
}

impl<T> ABLedgerWriter for &mut T
where
    T: ABLedgerWriter + ?Sized,
{
    fn write_ab_event(&mut self, event: &ABLedgerEvent) -> Result<()> {
        (**self).write_ab_event(event)
    }
}

impl<S, C> ABLedgerWriter for AnnealLedger<S, C>
where
    S: LedgerCfStore,
    C: Clock,
{
    fn write_ab_event(&mut self, event: &ABLedgerEvent) -> Result<()> {
        self.write(ab_ledger_entry(event)).map(|_| ())
    }
}

fn ab_ledger_entry(event: &ABLedgerEvent) -> AnnealLedgerEntry {
    AnnealLedgerEntry {
        action: event.action,
        change_id: event.record.change_id,
        artifact_id: artifact_id(&event.record.key.label()),
        prior_ptr_hash: event.prior_ptr_hash,
        candidate_ptr_hash: event.candidate_ptr_hash,
        metrics: MetricSnapshot {
            evaluated_at: event.record.ts,
            query_count: event.record.samples,
            metrics: vec![
                MetricComparison {
                    metric: TripwireMetric::SearchP99,
                    candidate_value: event.record.latency_after_ns as f64,
                    incumbent_value: event.record.latency_before_ns as f64,
                },
                MetricComparison {
                    metric: TripwireMetric::RecallAtK,
                    candidate_value: event.record.recall_after,
                    incumbent_value: event.record.recall_before,
                },
            ],
        },
        ts: event.record.ts,
        description: format!(
            "ab {} arm {} -> {} samples {} latency_before_ns {} latency_after_ns {} recall_before {:.6} recall_after {:.6} bits_before {:.6} bits_after {:.6} reason {}",
            ledger_safe_text(&event.record.key.label()),
            event.record.incumbent_idx,
            event.record.candidate_idx,
            event.record.samples,
            event.record.latency_before_ns,
            event.record.latency_after_ns,
            event.record.recall_before,
            event.record.recall_after,
            event.record.bits_before,
            event.record.bits_after,
            ledger_safe_text(&event.record.reason)
        ),
        fault: None,
        proposal: None,
        details: None,
        prev_hash: None,
    }
}

fn artifact_id(label: &str) -> String {
    blake3::hash(label.as_bytes()).to_hex().to_string()
}

fn ledger_safe_text(text: &str) -> String {
    text.chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}
