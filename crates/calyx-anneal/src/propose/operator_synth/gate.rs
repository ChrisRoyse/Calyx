use calyx_core::{Clock, Result};
use calyx_ledger::LedgerCfStore;
use serde_json::{Value, json};

use crate::{
    AnnealLedgerAction, AnnealLedgerActionPair, AnnealProposalLedgerOptions, AnnealSubstrate,
    ArtifactKey, ArtifactPtr, BudgetProbe, ChangeOutcome, RollbackStorage,
};

use super::{ANNEAL_OPERATOR_PROPOSAL_TAG, ProposedOperator, hex32, operator_label};

pub trait OperatorPromotionGate {
    fn ensure_operator_prior(&mut self, key: ArtifactKey, ptr: ArtifactPtr) -> Result<()>;

    fn propose_operator_change(
        &mut self,
        key: ArtifactKey,
        candidate_ptr: ArtifactPtr,
        details: Value,
        description: &str,
    ) -> Result<ChangeOutcome>;
}

impl<'a, R, L, C, P> OperatorPromotionGate for AnnealSubstrate<'a, R, L, C, P>
where
    R: RollbackStorage,
    L: LedgerCfStore,
    C: Clock,
    P: BudgetProbe,
{
    fn ensure_operator_prior(&mut self, key: ArtifactKey, ptr: ArtifactPtr) -> Result<()> {
        if self.rollback.live_ptr(&key)?.is_none() {
            self.rollback.install_live_ptr(key, ptr)?;
        }
        Ok(())
    }

    fn propose_operator_change(
        &mut self,
        key: ArtifactKey,
        candidate_ptr: ArtifactPtr,
        details: Value,
        description: &str,
    ) -> Result<ChangeOutcome> {
        self.propose_artifact_change_with_actions_and_details(
            key,
            candidate_ptr,
            AnnealProposalLedgerOptions::new(AnnealLedgerActionPair::new(
                AnnealLedgerAction::OperatorPromoted,
                AnnealLedgerAction::OperatorReverted,
            ))
            .with_details(details),
            description,
        )
    }
}

pub(super) fn operator_ledger_details(
    proposal_id: &str,
    operator: &ProposedOperator,
    deficit_total_bits: f64,
    refit_delta_j: f64,
    shadow_delta_j: f64,
) -> Value {
    let mut details = json!({
        "tag": ANNEAL_OPERATOR_PROPOSAL_TAG,
        "operator_id": proposal_id,
        "operator_kind": operator_label(operator),
        "deficit_total_bits": deficit_total_bits,
        "refit_delta_j": refit_delta_j,
        "shadow_delta_j": shadow_delta_j,
    });
    if let ProposedOperator::KernelScope { scope_hash, .. } = operator {
        details["scope_hash"] = json!(hex32(scope_hash));
    }
    details
}
