use std::collections::HashMap;

use calyx_core::{Constellation, Result};

use super::codec::{encode_head_rows, head_state_artifact_key, heads_hash};
use super::update::{update_reverted, validate_update};
use super::{
    HeadKind, HeadPromotionGate, HeadShadowProposal, HeadStorage, HeadUpdateOutcome, OnlineHead,
    OnlineHeadState, dot, summaries_from_maps,
};
use crate::{
    AnnealLedgerAction, AnnealSubstrate, ArtifactPtr, BudgetProbe, ChangeId, ChangeOutcome,
    FrozenLensCheck, MistakeLog, MistakeStorage, RegressionConfig, RegressionContextSource,
    RegressionPredictor, RegressionReport, ReplayEntry, RollbackStorage, assert_no_regression,
    regression_rate, regression_recurred,
};
use calyx_core::Clock;
use calyx_ledger::LedgerCfStore;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RegressionUpdateOutcome {
    pub update: HeadUpdateOutcome,
    pub report: RegressionReport,
}

pub trait HeadRegressionRollback {
    fn rollback_regressed_head_update(
        &mut self,
        change_id: ChangeId,
        report: &RegressionReport,
    ) -> Result<()>;
}

impl<'a, R, L, C, P> HeadRegressionRollback for AnnealSubstrate<'a, R, L, C, P>
where
    R: RollbackStorage,
    L: LedgerCfStore,
    C: Clock,
    P: BudgetProbe,
{
    fn rollback_regressed_head_update(
        &mut self,
        change_id: ChangeId,
        report: &RegressionReport,
    ) -> Result<()> {
        let rate = regression_rate(report)?;
        self.rollback_explicit_with_action(
            change_id,
            AnnealLedgerAction::HeadUpdateReverted,
            format!(
                "head update regression reassert failed regression_rate={rate:.6} regressions={} batch={}",
                report.regression_count,
                report.results.len()
            ),
        )
    }
}

impl<S, G, F> OnlineHeadState<S, G, F>
where
    S: HeadStorage,
    G: HeadPromotionGate + HeadRegressionRollback,
    F: FrozenLensCheck,
{
    pub fn update_with_regression<M, C>(
        &mut self,
        batch: &[ReplayEntry],
        log: &MistakeLog<M>,
        contexts: &C,
        lr: f32,
        fisher_weight: f32,
        config: RegressionConfig,
    ) -> Result<RegressionUpdateOutcome>
    where
        M: MistakeStorage,
        C: RegressionContextSource,
    {
        let config = config.validate()?;
        self.frozen_guard.assert_no_violation()?;
        validate_update(batch, lr, fisher_weight)?;
        if batch.is_empty() || lr == 0.0 || self.heads.is_empty() {
            let update = HeadUpdateOutcome {
                promoted: false,
                change_id: None,
                batch_len: batch.len(),
                updated_at: self.clock.now(),
                heads: self.summaries(),
            };
            return Ok(RegressionUpdateOutcome {
                update,
                report: RegressionReport::empty(),
            });
        }

        let candidate_heads = self.candidate_heads(batch, lr, fisher_weight)?;
        let candidate_map = candidate_heads
            .iter()
            .cloned()
            .map(|head| (head.kind, head))
            .collect::<HashMap<_, _>>();
        let report = assert_no_regression(
            &CandidatePredictor {
                heads: &candidate_map,
            },
            batch,
            log,
            contexts,
        )?;

        let prior_ptr = ArtifactPtr::ConfigCacheKeyHash(heads_hash(self.sorted_heads()?));
        let candidate_ptr = ArtifactPtr::ConfigCacheKeyHash(heads_hash(candidate_heads.clone()));
        let key = head_state_artifact_key();
        self.substrate.ensure_head_prior(key.clone(), prior_ptr)?;
        let rate = regression_rate(&report)?;
        let description = format!(
            "online_head_update batch={} lr={lr:.6} fisher_weight={fisher_weight:.6} regression_rate={rate:.6}",
            batch.len()
        );
        let proposal = HeadShadowProposal::stable();
        match self.substrate.propose_head_change(
            key,
            candidate_ptr,
            &proposal,
            &proposal,
            &description,
        )? {
            ChangeOutcome::Promoted(change_id) => {
                if config.exceeds(&report)? {
                    self.substrate
                        .rollback_regressed_head_update(change_id, &report)?;
                    return Err(regression_recurred(&report));
                }
                let rows = encode_head_rows(&candidate_heads)?;
                self.storage.save_heads(rows)?;
                let prior_heads = std::mem::take(&mut self.heads);
                self.heads = candidate_map;
                let update = HeadUpdateOutcome {
                    promoted: true,
                    change_id: Some(change_id),
                    batch_len: batch.len(),
                    updated_at: self.clock.now(),
                    heads: summaries_from_maps(&prior_heads, &self.heads),
                };
                self.frozen_guard.assert_no_violation()?;
                Ok(RegressionUpdateOutcome { update, report })
            }
            ChangeOutcome::Reverted { reason, .. } => Err(update_reverted(reason)),
        }
    }
}

impl<S, G, F> RegressionPredictor for OnlineHeadState<S, G, F>
where
    S: HeadStorage,
    G: HeadPromotionGate,
    F: FrozenLensCheck,
{
    fn predict_regression(&self, cx: &Constellation) -> f64 {
        self.predict(cx)
    }
}

struct CandidatePredictor<'a> {
    heads: &'a HashMap<HeadKind, OnlineHead>,
}

impl RegressionPredictor for CandidatePredictor<'_> {
    fn predict_regression(&self, cx: &Constellation) -> f64 {
        let Some(head) = self.heads.get(&HeadKind::Predictor) else {
            return 0.0;
        };
        dot(
            &head.params,
            &super::constellation_features(cx, head.params.len()),
        ) as f64
    }
}
