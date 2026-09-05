use crate::fs_tx::ObjectIdentity;
use crate::journal::db::{JournalTransaction, QueryExecutor, db_params};
use crate::protocol::RunId;

use super::super::{JournalError, ObservedRunState, RunState, TransactionState, TransitionUpdate};
use super::util::{sql, validate_optional_text};

pub(in crate::journal) fn require_active_run(
    transaction: &JournalTransaction<'_>,
    run_id: &RunId,
    operation: &'static str,
) -> Result<(), JournalError> {
    let state = transaction
        .query_optional(
            "SELECT state FROM runs WHERE run_id=?1",
            db_params![run_id.as_str()]?,
        )
        .map_err(|source| sql("read run authority", source))?;
    let state = state
        .map(|row| {
            row.get::<String>(0)
                .map_err(|source| sql("decode run authority", source))
        })
        .transpose()?;
    match state {
        Some(state) => {
            let actual = RunState::parse(&state)?;
            if actual == RunState::Active {
                Ok(())
            } else {
                Err(JournalError::RunNotActive {
                    operation,
                    run_id: run_id.clone(),
                    actual: ObservedRunState::Present(actual),
                })
            }
        }
        None => Err(JournalError::RunNotActive {
            operation,
            run_id: run_id.clone(),
            actual: ObservedRunState::Absent,
        }),
    }
}

pub(in crate::journal) fn validate_transition(
    from: TransactionState,
    to: TransactionState,
    update: &TransitionUpdate,
) -> Result<(), JournalError> {
    if !transaction_transition_allowed(from, to) {
        return Err(JournalError::InvalidMetadata(format!(
            "transition {from:?} -> {to:?} is forbidden"
        )));
    }
    if to == TransactionState::Prepared && update.identity.is_none() {
        return Err(JournalError::InvalidMetadata(
            "Prepared requires an object identity".into(),
        ));
    }
    if to == TransactionState::Quarantined
        && update.quarantine_name.as_deref().is_none_or(str::is_empty)
    {
        return Err(JournalError::InvalidMetadata(
            "Quarantined requires quarantine_name".into(),
        ));
    }
    if update.identity.is_some()
        && !matches!(
            to,
            TransactionState::Prepared | TransactionState::Quarantined
        )
    {
        return Err(JournalError::InvalidMetadata(
            "identity metadata is only valid for Prepared or Quarantined".into(),
        ));
    }
    if update.quarantine_name.is_some()
        && !matches!(
            to,
            TransactionState::Quarantined | TransactionState::MismatchPreserved
        )
    {
        return Err(JournalError::InvalidMetadata(
            "quarantine_name is only valid for Quarantined or MismatchPreserved".into(),
        ));
    }
    if matches!(
        to,
        TransactionState::MismatchPreserved | TransactionState::Failed
    ) && update.error_code.as_deref().is_none_or(str::is_empty)
    {
        return Err(JournalError::InvalidMetadata(
            "failure state requires error_code".into(),
        ));
    }
    if !matches!(
        to,
        TransactionState::MismatchPreserved | TransactionState::Failed
    ) && update.error_code.is_some()
    {
        return Err(JournalError::InvalidMetadata(
            "error_code is only valid for a failure state".into(),
        ));
    }
    validate_optional_text("quarantine_name", update.quarantine_name.as_deref(), 128)?;
    validate_optional_text("error_code", update.error_code.as_deref(), 96)?;
    validate_optional_text("detail", update.detail.as_deref(), 2_048)
}

pub(in crate::journal) fn transaction_transition_allowed(
    from: TransactionState,
    to: TransactionState,
) -> bool {
    matches!(
        (from, to),
        (
            TransactionState::Intent,
            TransactionState::Prepared | TransactionState::Failed
        ) | (
            TransactionState::Prepared,
            TransactionState::Published | TransactionState::Quarantined
        ) | (
            TransactionState::Published,
            TransactionState::Committed
                | TransactionState::DeleteIntent
                | TransactionState::Quarantined
                | TransactionState::MismatchPreserved
        ) | (TransactionState::Committed, TransactionState::DeleteIntent)
            | (
                TransactionState::DeleteIntent,
                TransactionState::Quarantined | TransactionState::MismatchPreserved
            )
            | (
                TransactionState::Quarantined,
                TransactionState::Deleted | TransactionState::MismatchPreserved
            )
    )
}

pub(in crate::journal) fn identity_fields(
    identity: &ObjectIdentity,
) -> (String, String, u32, u32, u32, i32, i32, Vec<u8>) {
    (
        identity.device.to_string(),
        identity.inode.to_string(),
        identity.owner_uid,
        identity.owner_gid,
        identity.mode,
        identity.opaque.mount_id(),
        identity.opaque.handle_type(),
        identity.opaque.bytes().to_vec(),
    )
}
