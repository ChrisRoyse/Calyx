use super::batch_support::{IdentityFields, identity_mismatch_reason};
use super::*;

pub(super) fn retained_text_input(vault_path: &std::path::Path, text: &str) -> CliResult<Input> {
    Ok(calyx_aster::retained_input::retain_text_input(
        vault_path, text,
    )?)
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
                    metadata: &existing.metadata,
                },
                IdentityFields {
                    panel_version: incoming.panel_version,
                    input_ref: &incoming.input_ref,
                    modality: incoming.modality,
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
) -> CliResult<bool> {
    let outcome = vault.backfill_input_pointer(cx_id, expected)?;
    let stored = vault.get(cx_id, vault.snapshot())?;
    if &stored.input_ref != expected {
        return Err(calyx_core::CalyxError::aster_corrupt_shard(format!(
            "retained input pointer readback for {cx_id} does not match the exact incoming input reference"
        ))
        .into());
    }
    ingest_runtime_log(format_args!(
        "phase=retained_input_pointer_readback cx_id={cx_id} changed={} ledger_seq={}",
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
