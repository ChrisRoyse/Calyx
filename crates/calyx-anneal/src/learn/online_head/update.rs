use calyx_core::{CalyxError, Clock, Result};
use calyx_ledger::LedgerCfStore;

use super::{HeadKind, OnlineHead, dot, invalid_row, validate_head};
use crate::CALYX_ANNEAL_HEAD_UPDATE_REVERTED;
use crate::{
    ActionMetricSnapshot, AnnealAction, AnnealLedgerAction, AnnealLedgerActionPair,
    AnnealSubstrate, ArtifactKey, ArtifactPtr, BudgetProbe, ChangeId, ChangeOutcome, ReplayEntry,
    RollbackStorage, ShadowRevertReason, TripwireMetric,
};

#[derive(Clone, Debug)]
pub struct HeadShadowProposal {
    metrics: ActionMetricSnapshot,
}

pub trait HeadPromotionGate {
    fn ensure_head_prior(&mut self, key: ArtifactKey, ptr: ArtifactPtr) -> Result<()>;
    fn propose_head_change(
        &mut self,
        key: ArtifactKey,
        candidate_ptr: ArtifactPtr,
        candidate: &HeadShadowProposal,
        incumbent: &HeadShadowProposal,
        description: &str,
    ) -> Result<ChangeOutcome>;
    fn record_sleep_pass_deferred(
        &mut self,
        _buffer_len: usize,
        _degraded_components: &[String],
    ) -> Result<()> {
        Ok(())
    }
    fn record_outcome_event(
        &mut self,
        _action: AnnealLedgerAction,
        _change_id: ChangeId,
        _artifact_id: String,
        _candidate_hash: [u8; 32],
        _description: String,
    ) -> Result<()> {
        Ok(())
    }
}

impl HeadShadowProposal {
    pub(crate) fn stable() -> Self {
        Self {
            metrics: ActionMetricSnapshot::from_values([
                (TripwireMetric::RecallAtK, 0.95),
                (TripwireMetric::GuardFAR, 0.001),
                (TripwireMetric::GuardFRR, 0.001),
                (TripwireMetric::SearchP99, 50.0),
                (TripwireMetric::IngestP95, 80.0),
            ]),
        }
    }
}

impl AnnealAction for HeadShadowProposal {
    fn apply_shadow(&self, _query: &crate::ReplayQuery) -> ActionMetricSnapshot {
        self.metrics.clone()
    }
}

impl<'a, R, L, C, P> HeadPromotionGate for AnnealSubstrate<'a, R, L, C, P>
where
    R: RollbackStorage,
    L: LedgerCfStore,
    C: Clock,
    P: BudgetProbe,
{
    fn ensure_head_prior(&mut self, key: ArtifactKey, ptr: ArtifactPtr) -> Result<()> {
        if self.rollback.live_ptr(&key)?.is_none() {
            self.rollback.install_live_ptr(key, ptr)?;
        }
        Ok(())
    }

    fn propose_head_change(
        &mut self,
        key: ArtifactKey,
        candidate_ptr: ArtifactPtr,
        candidate: &HeadShadowProposal,
        incumbent: &HeadShadowProposal,
        description: &str,
    ) -> Result<ChangeOutcome> {
        self.propose_change_with_actions(
            key,
            candidate_ptr,
            candidate,
            incumbent,
            AnnealLedgerActionPair::new(
                AnnealLedgerAction::HeadUpdate,
                AnnealLedgerAction::HeadUpdateReverted,
            ),
            description,
        )
    }

    fn record_sleep_pass_deferred(
        &mut self,
        buffer_len: usize,
        degraded_components: &[String],
    ) -> Result<()> {
        let components = if degraded_components.is_empty() {
            "none".to_string()
        } else {
            degraded_components.join("; ")
        };
        self.write_sleep_pass_deferred(format!(
            "sleep pass deferred degraded_count={} buffer_len={} components={components}",
            degraded_components.len(),
            buffer_len
        ))
    }

    fn record_outcome_event(
        &mut self,
        action: AnnealLedgerAction,
        change_id: ChangeId,
        artifact_id: String,
        candidate_hash: [u8; 32],
        description: String,
    ) -> Result<()> {
        self.write_outcome_event(action, change_id, artifact_id, candidate_hash, description)
    }
}

pub(crate) fn apply_update(
    head: &OnlineHead,
    batch: &[ReplayEntry],
    lr: f32,
    fisher_weight: f32,
) -> Result<OnlineHead> {
    let len = head.params.len();
    let mut gradient = vec![0.0_f32; len];
    let mut observed_fisher = vec![0.0_f32; len];
    let scale = 1.0 / batch.len() as f32;
    for entry in batch {
        let features = entry_features(head.kind, len, entry);
        let prediction = dot(&head.params, &features);
        let target = entry.surprise as f32;
        let error = prediction - target;
        for index in 0..len {
            let partial = error * features[index];
            gradient[index] += partial * scale;
            observed_fisher[index] += partial * partial * scale;
        }
    }
    let mut next = head.clone();
    for index in 0..len {
        let prior = next.prior_params[index];
        let regularizer = fisher_weight * next.fisher_diag[index] * (next.params[index] - prior);
        next.params[index] += -lr * (gradient[index] + regularizer);
        next.fisher_diag[index] = next.fisher_diag[index].max(observed_fisher[index]);
    }
    next.version = next
        .version
        .checked_add(1)
        .ok_or_else(|| invalid_row("online head version exhausted"))?;
    validate_head(&next)?;
    Ok(next)
}

pub(crate) fn validate_update(batch: &[ReplayEntry], lr: f32, fisher_weight: f32) -> Result<()> {
    if !lr.is_finite() || lr < 0.0 {
        return Err(invalid_row(
            "online head learning rate must be finite and >= 0",
        ));
    }
    if !fisher_weight.is_finite() || fisher_weight < 0.0 {
        return Err(invalid_row(
            "online head fisher_weight must be finite and >= 0",
        ));
    }
    for entry in batch {
        if !entry.surprise.is_finite() || entry.surprise < 0.0 {
            return Err(invalid_row("online head batch contains invalid surprise"));
        }
    }
    Ok(())
}

pub(crate) fn update_reverted(reason: ShadowRevertReason) -> CalyxError {
    CalyxError {
        code: CALYX_ANNEAL_HEAD_UPDATE_REVERTED,
        message: format!("online head update reverted by substrate: {reason:?}"),
        remediation: "inspect anneal rollback and tripwire rows before retrying",
    }
}

fn entry_features(kind: HeadKind, len: usize, entry: &ReplayEntry) -> Vec<f32> {
    let mut features = Vec::with_capacity(len);
    let bytes = entry.cx_id.to_bytes();
    for index in 0..len {
        let value = match index {
            0 => 1.0,
            1 => entry.surprise as f32,
            2 => entry.mistake_ref.surprise as f32,
            3 => entry.mistake_ref.seq as f32 / (entry.mistake_ref.seq as f32 + 1.0),
            _ => bytes[(index - 4) % bytes.len()] as f32 / 255.0,
        };
        features.push(match kind {
            HeadKind::Predictor => value,
            HeadKind::Calibrator => value * 0.5,
            HeadKind::FusionWeights => value / len.max(1) as f32,
        });
    }
    features
}
