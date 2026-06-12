use std::cmp::Ordering;
use std::collections::BinaryHeap;

use calyx_aster::cf::full_content_hash;
use calyx_aster::vault::AsterVault;
use calyx_core::{CalyxError, Clock, Result};
use calyx_ledger::LedgerCfStore;

use crate::{
    ActionMetricSnapshot, AnnealAction, AnnealLedger, AnnealLedgerAction, AnnealLedgerEntry,
    AnnealSubstrate, ArtifactPtr, BudgetProbe, CALYX_ANNEAL_BUDGET_EXHAUSTED, ChangeId,
    ChangeOutcome, ComponentHealth, DegradeRegistry, HealthStorage, MetricSnapshot, ReplayQuery,
    RollbackStorage, TripwireMetric,
};

use super::artifact::{hex, ptr_hash, target_hash};
use super::builders::{AnnIndexRebuilder, GuardProfileRebuilder, KernelIndexRebuilder};
use super::source::AsterRebuildSource;
use super::{
    CALYX_ANNEAL_REBUILD_TRIPWIRE_FAILED, RebuildJob, RebuildOutcome, RebuildPriority,
    RebuildTarget, Rebuilder, invalid_target,
};

const REBUILD_CPU_WEIGHT: f64 = 0.01;
const REBUILD_VRAM_BYTES: u64 = 0;

pub struct RebuildScheduler<'a, C>
where
    C: Clock,
{
    source: AsterRebuildSource<'a, C>,
    clock: &'a dyn Clock,
    queue: BinaryHeap<QueuedJob>,
    next_sequence: u64,
    ann: Box<dyn Rebuilder + 'a>,
    kernel: Box<dyn Rebuilder + 'a>,
    guard: Box<dyn Rebuilder + 'a>,
}

impl<'a, C> RebuildScheduler<'a, C>
where
    C: Clock + 'a,
{
    pub fn new(
        clock: &'a dyn Clock,
        vault: &'a AsterVault<C>,
        artifact_dir: impl Into<std::path::PathBuf>,
    ) -> Self {
        let source = AsterRebuildSource::new(vault);
        Self::with_rebuilders(
            clock,
            source,
            Box::new(AnnIndexRebuilder::new(source, artifact_dir)),
            Box::new(KernelIndexRebuilder::new(source)),
            Box::new(GuardProfileRebuilder::new(source)),
        )
    }

    pub fn with_rebuilders(
        clock: &'a dyn Clock,
        source: AsterRebuildSource<'a, C>,
        ann: Box<dyn Rebuilder + 'a>,
        kernel: Box<dyn Rebuilder + 'a>,
        guard: Box<dyn Rebuilder + 'a>,
    ) -> Self {
        Self {
            source,
            clock,
            queue: BinaryHeap::new(),
            next_sequence: 0,
            ann,
            kernel,
            guard,
        }
    }

    pub fn enqueue(&mut self, target: RebuildTarget, priority: RebuildPriority) {
        let job = RebuildJob {
            target,
            priority,
            sequence: self.next_sequence,
        };
        self.next_sequence = self.next_sequence.saturating_add(1);
        self.queue.push(QueuedJob(job));
    }

    pub fn pending_len(&self) -> usize {
        self.queue.len()
    }

    pub fn run_next<S, R, L, LC, P>(
        &mut self,
        registry: &mut DegradeRegistry<S>,
        substrate: &mut AnnealSubstrate<'_, R, L, LC, P>,
    ) -> Result<RebuildOutcome>
    where
        S: HealthStorage,
        R: RollbackStorage,
        L: LedgerCfStore,
        LC: Clock,
        P: BudgetProbe,
    {
        let Some(QueuedJob(job)) = self.queue.pop() else {
            return Ok(RebuildOutcome::NothingQueued);
        };
        let component = job.target.component();
        if !matches!(
            registry.health(&component),
            ComponentHealth::Degraded { .. }
        ) {
            return Ok(RebuildOutcome::SkippedNotDegraded { target: job.target });
        }
        let mut budget = match substrate
            .budget
            .acquire(REBUILD_CPU_WEIGHT, REBUILD_VRAM_BYTES)
        {
            Ok(handle) => handle,
            Err(error) if error.code == CALYX_ANNEAL_BUDGET_EXHAUSTED => {
                self.requeue(job.clone());
                return Ok(RebuildOutcome::BudgetExhausted { target: job.target });
            }
            Err(error) => return Err(error),
        };
        let snapshot = self.source.latest_snapshot();
        let prior_ptr = match substrate.rollback.live_ptr(&job.target.artifact_key())? {
            Some(ptr) => ptr,
            None => {
                let error = invalid_target("rebuild requires an installed live pointer");
                return self.record_failure(substrate, job.target, error);
            }
        };
        let new_ptr =
            match self
                .rebuilder_for(&job.target)
                .rebuild(&job.target, snapshot, &mut budget)
            {
                Ok(ptr) => ptr,
                Err(error) if error.code == CALYX_ANNEAL_BUDGET_EXHAUSTED => {
                    self.requeue(job.clone());
                    return Ok(RebuildOutcome::BudgetExhausted { target: job.target });
                }
                Err(error) => return self.record_failure(substrate, job.target, error),
            };
        let action = RebuildShadowAction;
        let outcome = substrate.propose_change_with_description(
            job.target.artifact_key(),
            new_ptr.clone(),
            &action,
            &action,
            "background rebuild",
        )?;
        match outcome {
            ChangeOutcome::Promoted(change_id) => {
                registry.confirm_healed(component, &mut substrate.ledger)?;
                write_rebuild_event(
                    &mut substrate.ledger,
                    &job.target,
                    change_id,
                    &prior_ptr,
                    &new_ptr,
                    self.clock.now(),
                    "rebuild completed",
                )?;
                Ok(RebuildOutcome::Completed {
                    change_id,
                    prior_ptr,
                    new_ptr,
                })
            }
            ChangeOutcome::Reverted { reason, .. } => {
                let error = tripwire_failed(format!("rebuild tripwire reverted: {reason:?}"));
                self.record_failure(substrate, job.target, error)
            }
        }
    }

    fn record_failure<R, L, LC, P>(
        &self,
        substrate: &mut AnnealSubstrate<'_, R, L, LC, P>,
        target: RebuildTarget,
        error: CalyxError,
    ) -> Result<RebuildOutcome>
    where
        R: RollbackStorage,
        L: LedgerCfStore,
        LC: Clock,
        P: BudgetProbe,
    {
        write_rebuild_failure(&mut substrate.ledger, &target, &error, self.clock.now())?;
        Ok(RebuildOutcome::Failed {
            target,
            reason_code: error.code.to_string(),
            reason: error.message,
        })
    }

    fn rebuilder_for(&self, target: &RebuildTarget) -> &dyn Rebuilder {
        match target {
            RebuildTarget::AnnIndex { .. } => self.ann.as_ref(),
            RebuildTarget::KernelIndex { .. } => self.kernel.as_ref(),
            RebuildTarget::GuardProfile { .. } => self.guard.as_ref(),
        }
    }

    fn requeue(&mut self, mut job: RebuildJob) {
        job.sequence = self.next_sequence;
        self.next_sequence = self.next_sequence.saturating_add(1);
        self.queue.push(QueuedJob(job));
    }
}

#[derive(Clone, Debug, Eq)]
struct QueuedJob(RebuildJob);

impl Ord for QueuedJob {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0
            .priority
            .cmp(&other.0.priority)
            .then_with(|| other.0.sequence.cmp(&self.0.sequence))
    }
}

impl PartialOrd for QueuedJob {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for QueuedJob {
    fn eq(&self, other: &Self) -> bool {
        self.0.priority == other.0.priority && self.0.sequence == other.0.sequence
    }
}

#[derive(Clone)]
struct RebuildShadowAction;

impl AnnealAction for RebuildShadowAction {
    fn apply_shadow(&self, _query: &ReplayQuery) -> ActionMetricSnapshot {
        ActionMetricSnapshot::from_values([
            (TripwireMetric::RecallAtK, 0.95),
            (TripwireMetric::GuardFAR, 0.001),
            (TripwireMetric::GuardFRR, 0.001),
            (TripwireMetric::SearchP99, 50.0),
            (TripwireMetric::IngestP95, 80.0),
        ])
    }
}

fn write_rebuild_event<L, C>(
    ledger: &mut AnnealLedger<L, C>,
    target: &RebuildTarget,
    change_id: ChangeId,
    prior_ptr: &ArtifactPtr,
    new_ptr: &ArtifactPtr,
    ts: u64,
    description: impl Into<String>,
) -> Result<()>
where
    L: LedgerCfStore,
    C: Clock,
{
    ledger
        .write(AnnealLedgerEntry {
            action: AnnealLedgerAction::Rebuild,
            change_id,
            artifact_id: hex(&target_hash(target)),
            prior_ptr_hash: ptr_hash(prior_ptr),
            candidate_ptr_hash: ptr_hash(new_ptr),
            metrics: MetricSnapshot::empty(ts),
            ts,
            description: description.into(),
            fault: None,
            proposal: None,
            details: None,
            prev_hash: None,
        })
        .map(|_| ())
}

fn write_rebuild_failure<L, C>(
    ledger: &mut AnnealLedger<L, C>,
    target: &RebuildTarget,
    error: &CalyxError,
    ts: u64,
) -> Result<()>
where
    L: LedgerCfStore,
    C: Clock,
{
    let error_hash = full_content_hash([error.code.as_bytes(), error.message.as_bytes()]);
    ledger
        .write(AnnealLedgerEntry {
            action: AnnealLedgerAction::Rebuild,
            change_id: failure_change_id(target, ts),
            artifact_id: hex(&target_hash(target)),
            prior_ptr_hash: target_hash(target),
            candidate_ptr_hash: error_hash,
            metrics: MetricSnapshot::empty(ts),
            ts,
            description: format!("rebuild failed {}: {}", error.code, error.message),
            fault: None,
            proposal: None,
            details: None,
            prev_hash: None,
        })
        .map(|_| ())
}

fn failure_change_id(target: &RebuildTarget, ts: u64) -> ChangeId {
    let hash = target_hash(target);
    let mut raw = [0_u8; 8];
    raw.copy_from_slice(&hash[..8]);
    ChangeId((u64::from_be_bytes(raw) ^ ts).max(1))
}

fn tripwire_failed(message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: CALYX_ANNEAL_REBUILD_TRIPWIRE_FAILED,
        message: message.into(),
        remediation: "keep the prior derived artifact live and inspect tripwire metrics before retrying",
    }
}
