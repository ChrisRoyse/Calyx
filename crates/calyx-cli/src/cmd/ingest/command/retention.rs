use super::batch_support::{IdentityFields, identity_mismatch_reason};
use super::*;

pub(super) fn prepared_text_input(text: &str) -> Input {
    calyx_aster::retained_input::prepare_text_input(text)
}

pub(super) fn prepared_hybrid_text_input(
    text: &str,
    record: Option<&calyx_core::StructuredRecord>,
) -> Input {
    let input = prepared_text_input(text);
    match record {
        Some(record) => input.with_structured(record.clone()),
        None => input,
    }
}

pub(super) fn preflight_existing_text_identity(
    vault: &AsterVault,
    state: &VaultPanelState,
    incoming: &Constellation,
) -> CliResult<()> {
    let existing = vault.get(incoming.cx_id, vault.snapshot())?;
    if existing.panel_version != incoming.panel_version
        || !input_ref_matches_or_backfillable(&existing.input_ref, &incoming.input_ref)
        || existing.modality != incoming.modality
        || existing.scalars != incoming.scalars
        || existing.metadata != incoming.metadata
    {
        return Err(CliError::usage(format!(
            "idempotent text replay for cx {} changed stored identity: {}",
            incoming.cx_id,
            identity_mismatch_reason(
                IdentityFields {
                    panel_version: existing.panel_version,
                    input_ref: &existing.input_ref,
                    modality: existing.modality,
                    scalars: &existing.scalars,
                    metadata: &existing.metadata,
                },
                IdentityFields {
                    panel_version: incoming.panel_version,
                    input_ref: &incoming.input_ref,
                    modality: incoming.modality,
                    scalars: &incoming.scalars,
                    metadata: &incoming.metadata,
                },
            )
        )));
    }
    ensure_content_panel_floor(&existing, state)
}

pub(super) fn apply_existing_input_pointer(
    vault: &AsterVault,
    cx_id: CxId,
    expected: &InputRef,
    bytes: &[u8],
    intent: &mut dyn calyx_aster::vault::DurableCommitIntent,
) -> CliResult<bool> {
    let outcome = vault.backfill_input_pointer_with_retained_input_and_commit_intent(
        cx_id, expected, bytes, intent,
    )?;
    ingest_runtime_log(format_args!(
        "phase=retained_input_pointer_commit_outcome cx_id={cx_id} changed={} ledger_seq={}",
        outcome.changed(),
        outcome.ledger_ref().seq
    ));
    Ok(outcome.changed())
}

pub(super) fn input_ref_matches_or_backfillable(existing: &InputRef, incoming: &InputRef) -> bool {
    existing.hash == incoming.hash
        && existing.redacted == incoming.redacted
        && (existing.pointer == incoming.pointer
            || (existing.pointer.is_none() && incoming.pointer.is_some()))
}
