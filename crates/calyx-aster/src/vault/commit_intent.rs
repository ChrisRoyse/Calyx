//! Side-effect intents serialized with Aster's authoritative durable commit.

use super::AsterVault;
use crate::cf::ColumnFamily;
use calyx_core::{CalyxError, Clock, Result, Seq};

pub const CALYX_DURABLE_COMMIT_INTENT_RECONCILIATION_REQUIRED: &str =
    "CALYX_DURABLE_COMMIT_INTENT_RECONCILIATION_REQUIRED";

/// A durable side effect whose lifecycle must exactly bracket an Aster commit.
///
/// Aster calls `prepare` while holding the durable commit lock and immediately
/// before the operation can mutate storage. It then reports either the exact
/// committed sequence or a proven no-commit abort before releasing that lock.
/// Implementations must fail closed when their own persisted state is not the
/// exact state they prepared.
pub trait DurableCommitIntent {
    fn prepare(&mut self, predicted_seq: Seq) -> Result<()>;
    fn committed(&mut self, committed_seq: Seq) -> Result<()>;
    fn aborted(&mut self) -> Result<()>;
}

#[derive(Default)]
pub struct NoopDurableCommitIntent;

impl DurableCommitIntent for NoopDurableCommitIntent {
    fn prepare(&mut self, _predicted_seq: Seq) -> Result<()> {
        Ok(())
    }

    fn committed(&mut self, _committed_seq: Seq) -> Result<()> {
        Ok(())
    }

    fn aborted(&mut self) -> Result<()> {
        Ok(())
    }
}

impl<C> AsterVault<C>
where
    C: Clock,
{
    pub(crate) fn run_with_commit_intent_locked<T>(
        &self,
        intent: &mut dyn DurableCommitIntent,
        operation: impl FnOnce() -> Result<T>,
    ) -> Result<T> {
        let before = self.commit_intent_observed_seq()?;
        let predicted = before.checked_add(1).ok_or_else(|| {
            commit_intent_invariant(
                "durable commit sequence overflow while preparing a commit intent",
            )
        })?;
        intent.prepare(predicted)?;
        let operation_result = operation();
        let after = match self.commit_intent_observed_seq() {
            Ok(after) => after,
            Err(readback) => {
                return Err(intent_reconciliation_error(
                    before,
                    None,
                    operation_result.as_ref().err(),
                    &readback,
                ));
            }
        };
        if after < before {
            return Err(intent_reconciliation_error(
                before,
                Some(after),
                operation_result.as_ref().err(),
                &commit_intent_invariant(
                    "durable commit sequence regressed while reconciling a commit intent",
                ),
            ));
        }

        match operation_result {
            Ok(value) if after == before => {
                intent.aborted().map_err(|intent_error| {
                    intent_reconciliation_error(before, Some(after), None, &intent_error)
                })?;
                Ok(value)
            }
            Ok(value) => {
                intent.committed(after).map_err(|intent_error| {
                    intent_reconciliation_error(before, Some(after), None, &intent_error)
                })?;
                Ok(value)
            }
            Err(operation_error) if after == before => {
                intent.aborted().map_err(|intent_error| {
                    intent_reconciliation_error(
                        before,
                        Some(after),
                        Some(&operation_error),
                        &intent_error,
                    )
                })?;
                Err(operation_error)
            }
            Err(operation_error) => {
                intent.committed(after).map_err(|intent_error| {
                    intent_reconciliation_error(
                        before,
                        Some(after),
                        Some(&operation_error),
                        &intent_error,
                    )
                })?;
                Err(operation_error)
            }
        }
    }

    /// Serializes publication/finalization of a derived generation with Base
    /// commits and refuses to publish against a stale snapshot.
    pub fn with_derived_generation_fence<T>(
        &self,
        built_at_base_seq: Seq,
        operation: impl FnOnce() -> Result<T>,
    ) -> Result<T> {
        self.with_durable_commit_lock(|| {
            let derived_content_seq = self.derived_content_seq();
            if derived_content_seq > built_at_base_seq {
                return Err(CalyxError::stale_derived(format!(
                    "derived generation was built at base seq {built_at_base_seq}, but search-relevant Base state advanced to seq {derived_content_seq} before publication"
                )));
            }
            operation()
        })
    }

    /// Writes raw CF rows while serializing an external durable intent with
    /// the exact Aster commit. Empty input performs no intent transition.
    pub fn write_cf_batch_with_commit_intent(
        &self,
        rows: impl IntoIterator<Item = (ColumnFamily, Vec<u8>, Vec<u8>)>,
        intent: &mut dyn DurableCommitIntent,
    ) -> Result<Seq> {
        let rows = rows
            .into_iter()
            .map(|(cf, key, value)| super::encode::WriteRow { cf, key, value })
            .collect::<Vec<_>>();
        if rows.is_empty() {
            return Ok(self.latest_seq());
        }
        self.with_durable_commit_lock(|| {
            self.run_with_commit_intent_locked(intent, || self.commit_rows_locked(&rows))
        })
    }

    fn commit_intent_observed_seq(&self) -> Result<Seq> {
        match &self.durable {
            Some(durable) => durable.durable_tip_seq(),
            None => Ok(self.latest_seq()),
        }
    }
}

fn intent_reconciliation_error(
    before: Seq,
    after: Option<Seq>,
    operation: Option<&CalyxError>,
    intent: &CalyxError,
) -> CalyxError {
    CalyxError {
        code: CALYX_DURABLE_COMMIT_INTENT_RECONCILIATION_REQUIRED,
        message: format!(
            "durable commit intent could not be reconciled: seq_before={before} seq_after={} operation={} intent_error[{}]: {}",
            after
                .map(|seq| seq.to_string())
                .unwrap_or_else(|| "unreadable".to_string()),
            operation
                .map(|error| format!("error[{}]: {}", error.code, error.message))
                .unwrap_or_else(|| "ok".to_string()),
            intent.code,
            intent.message,
        ),
        remediation: "preserve the durable intent, reopen the vault, read the authoritative Base/WAL sequence and side-effect state, then reconcile before retrying",
    }
}

fn commit_intent_invariant(message: impl Into<String>) -> CalyxError {
    CalyxError {
        code: CALYX_DURABLE_COMMIT_INTENT_RECONCILIATION_REQUIRED,
        message: message.into(),
        remediation: "preserve the durable intent, reopen the vault, read the authoritative Base/WAL sequence and side-effect state, then reconcile before retrying",
    }
}
