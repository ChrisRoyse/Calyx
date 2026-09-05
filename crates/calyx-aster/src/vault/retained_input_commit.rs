//! Aster-owned commit boundary for exact retained source bytes.

use calyx_core::{CalyxError, Clock, Constellation, CxId, InputRef, Result, VaultStore};

use super::{AsterVault, DurableCommitIntent, NoopDurableCommitIntent, PutOutcome};
use crate::retained_input::{
    CALYX_INPUT_RETENTION_RECONCILIATION_REQUIRED, CALYX_INPUT_RETENTION_REQUIRES_DURABLE_VAULT,
    PendingRetentionBatch, reconcile_pending_retained_inputs, rollback_error,
};

impl<C> AsterVault<C>
where
    C: Clock,
{
    /// Commits constellations and publishes their exact retained bytes as one
    /// recoverable operation under the durable commit lock.
    pub fn put_batch_with_retained_inputs<I>(&self, inputs: I) -> Result<Vec<PutOutcome>>
    where
        I: IntoIterator<Item = (Constellation, Vec<u8>)>,
    {
        let mut intent = NoopDurableCommitIntent;
        self.put_batch_with_retained_inputs_and_commit_intent(inputs, &mut intent)
    }

    /// Commits constellations and retained bytes while an external durable
    /// intent is serialized with the exact Aster commit sequence.
    pub fn put_batch_with_retained_inputs_and_commit_intent<I>(
        &self,
        inputs: I,
        intent: &mut dyn DurableCommitIntent,
    ) -> Result<Vec<PutOutcome>>
    where
        I: IntoIterator<Item = (Constellation, Vec<u8>)>,
    {
        let inputs = inputs.into_iter().collect::<Vec<_>>();
        if inputs.is_empty() {
            return Ok(Vec::new());
        }
        let vault_root = self.durable_root()?.to_path_buf();
        self.with_durable_commit_lock(|| {
            let durable_before = self.retained_durable_tip()?;
            let pending = PendingRetentionBatch::stage(
                &vault_root,
                inputs
                    .iter()
                    .map(|(cx, bytes)| (&cx.input_ref, bytes.as_slice())),
            )?;
            let constellations = inputs.into_iter().map(|(cx, _)| cx).collect::<Vec<_>>();
            let operation = self.run_with_commit_intent_locked(intent, || {
                self.put_batch_locked_retained_outcomes(constellations)
            });
            self.finish_retained_input_commit(durable_before, pending, operation)
        })
    }

    /// Recreates or verifies an already-referenced input from exact replay
    /// bytes without changing Base or Ledger state.
    pub fn ensure_retained_input(&self, id: CxId, expected: &InputRef, bytes: &[u8]) -> Result<()> {
        let vault_root = self.durable_root()?.to_path_buf();
        self.with_durable_commit_lock(|| {
            let stored = self.get(id, self.snapshot())?;
            if &stored.input_ref != expected {
                return Err(CalyxError {
                    code: super::CALYX_INPUT_POINTER_IDENTITY_MISMATCH,
                    message: format!(
                        "cannot ensure retained input for {id}: stored input reference differs from the exact replay reference"
                    ),
                    remediation: "supply the exact authoritative bytes and input reference already stored for this constellation",
                });
            }
            PendingRetentionBatch::stage(&vault_root, [(expected, bytes)])?.finalize()
        })
    }

    pub(crate) fn reconcile_retained_input_state(&self) -> Result<()> {
        let vault_root = self.durable_root()?.to_path_buf();
        self.with_durable_commit_lock(|| reconcile_pending_retained_inputs(self, &vault_root))
    }

    pub(super) fn durable_root(&self) -> Result<&std::path::Path> {
        self.durable
            .as_ref()
            .map(|durable| durable.root())
            .ok_or_else(|| CalyxError {
                code: CALYX_INPUT_RETENTION_REQUIRES_DURABLE_VAULT,
                message: "retained input commit requires a durable Aster vault".to_string(),
                remediation: "open the target vault from its durable directory before retaining source bytes",
            })
    }

    pub(super) fn retained_durable_tip(&self) -> Result<u64> {
        self.durable
            .as_ref()
            .ok_or_else(|| CalyxError {
                code: CALYX_INPUT_RETENTION_REQUIRES_DURABLE_VAULT,
                message: "retained input commit requires a durable Aster vault".to_string(),
                remediation: "open the target vault from its durable directory before retaining source bytes",
            })?
            .durable_tip_seq()
    }

    pub(super) fn finish_retained_input_commit<T>(
        &self,
        durable_before: u64,
        pending: PendingRetentionBatch,
        operation: Result<T>,
    ) -> Result<T> {
        match operation {
            Ok(value) => match pending.finalize() {
                Ok(()) => Ok(value),
                Err(publication) => Err(retention_reconciliation_error(format!(
                    "Aster commit succeeded after durable seq {durable_before}, but retained-input publication failed with {}: {}",
                    publication.code, publication.message
                ))),
            },
            Err(operation) => match self.retained_durable_tip() {
                Ok(durable_after) if durable_after == durable_before => match pending.rollback() {
                    Ok(()) => Err(operation),
                    Err(rollback) => Err(rollback_error(&operation, &rollback)),
                },
                Ok(durable_after) if durable_after > durable_before => match pending.finalize() {
                    Ok(()) => Err(operation),
                    Err(publication) => Err(retention_reconciliation_error(format!(
                        "Aster operation returned {}: {} after durable seq advanced {durable_before}->{durable_after}; retained-input publication also failed with {}: {}",
                        operation.code, operation.message, publication.code, publication.message
                    ))),
                },
                Ok(durable_after) => Err(retention_reconciliation_error(format!(
                    "Aster operation returned {}: {}, and durable seq regressed {durable_before}->{durable_after}; pending retained inputs were preserved",
                    operation.code, operation.message
                ))),
                Err(readback) => Err(retention_reconciliation_error(format!(
                    "Aster operation returned {}: {}; durable commit readback failed with {}: {}; pending retained inputs were preserved",
                    operation.code, operation.message, readback.code, readback.message
                ))),
            },
        }
    }
}

fn retention_reconciliation_error(message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: CALYX_INPUT_RETENTION_RECONCILIATION_REQUIRED,
        message: message.into(),
        remediation: "reopen the write-capable vault, then verify the Base row and exact retained bytes before retrying",
    }
}
