use std::sync::Arc;

use calyx_core::{Clock, Result};
use calyx_ledger::LedgerCfStore;

use crate::shadow::ArtifactShadowAction;
use crate::{
    AnnealLedgerAction, AnnealLedgerActionPair, AnnealProposalLedgerOptions, AnnealSubstrate,
    ArtifactKey, ArtifactPtr, ArtifactReplayMeasurer, BudgetProbe, ChangeOutcome, RollbackStorage,
};

impl<'a, R, L, C, P> AnnealSubstrate<'a, R, L, C, P>
where
    R: RollbackStorage,
    L: LedgerCfStore,
    C: Clock,
    P: BudgetProbe,
{
    pub fn with_replay_measurer(mut self, measurer: Arc<dyn ArtifactReplayMeasurer>) -> Self {
        self.replay_measurer = Some(measurer);
        self
    }

    pub fn propose_artifact_change_with_description(
        &mut self,
        key: ArtifactKey,
        candidate_ptr: ArtifactPtr,
        description: impl Into<String>,
    ) -> Result<ChangeOutcome> {
        self.propose_artifact_change_with_actions(
            key,
            candidate_ptr,
            AnnealLedgerActionPair::new(AnnealLedgerAction::Promote, AnnealLedgerAction::Revert),
            description,
        )
    }

    pub fn propose_artifact_change_with_actions(
        &mut self,
        key: ArtifactKey,
        candidate_ptr: ArtifactPtr,
        actions: AnnealLedgerActionPair,
        description: impl Into<String>,
    ) -> Result<ChangeOutcome> {
        self.propose_artifact_change_with_actions_and_details(
            key,
            candidate_ptr,
            AnnealProposalLedgerOptions::new(actions),
            description,
        )
    }

    pub fn propose_artifact_change_with_actions_and_details(
        &mut self,
        key: ArtifactKey,
        candidate_ptr: ArtifactPtr,
        ledger_options: AnnealProposalLedgerOptions,
        description: impl Into<String>,
    ) -> Result<ChangeOutcome> {
        let description = description.into();
        let (change_id, readback) =
            self.prepare_change(key.clone(), candidate_ptr.clone(), description.as_str())?;
        let incumbent_ptr = readback.snapshot.prior_ptr.clone();
        let candidate = ArtifactShadowAction::new(
            key.clone(),
            candidate_ptr.clone(),
            self.replay_measurer.clone(),
        );
        let incumbent =
            ArtifactShadowAction::new(key.clone(), incumbent_ptr, self.replay_measurer.clone());
        let verdict = self.shadow_verdict(&candidate, &incumbent)?;
        self.finish_prepared_change(change_id, readback, verdict, ledger_options, description)
    }
}
