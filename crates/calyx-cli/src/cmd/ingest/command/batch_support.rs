use super::replay::ExistingBatchReplayRow;
use super::*;

pub(crate) struct IdentityFields<'a> {
    pub(crate) panel_version: u32,
    pub(crate) input_ref: &'a InputRef,
    pub(crate) modality: Modality,
    pub(crate) metadata: &'a BTreeMap<String, String>,
}
pub(crate) fn append_idempotent_batch_ledger(
    vault: &AsterVault,
    order: &[BatchOrderRow],
) -> CliResult<Option<u64>> {
    let ids = order
        .iter()
        .filter(|row| !row.new)
        .map(|row| row.cx_id)
        .collect::<Vec<_>>();
    if ids.is_empty() {
        return Ok(None);
    }
    append_cli_batch_ledger(
        vault,
        EntryKind::Ingest,
        &ids,
        "cli-idempotent-ingest-batch",
    )
    .map(Some)
}

pub(crate) fn verify_existing_batch_replay_identity(
    vault: &AsterVault,
    state: &VaultPanelState,
    row: &ExistingBatchReplayRow,
) -> CliResult<Constellation> {
    let existing = vault.get(row.cx_id, vault.snapshot())?;
    if existing.panel_version != state.panel.version
        || existing.input_ref != row.input_ref
        || existing.modality != row.modality
        || existing.metadata != row.metadata
    {
        return Err(CliError::usage(format!(
            "idempotent batch replay for cx {} changed stored non-anchor identity: {}",
            row.cx_id,
            identity_mismatch_reason(
                IdentityFields {
                    panel_version: existing.panel_version,
                    input_ref: &existing.input_ref,
                    modality: existing.modality,
                    metadata: &existing.metadata,
                },
                IdentityFields {
                    panel_version: state.panel.version,
                    input_ref: &row.input_ref,
                    modality: row.modality,
                    metadata: &row.metadata,
                },
            )
        )));
    }
    ensure_content_panel_floor(&existing, state)?;
    Ok(existing)
}

pub(crate) fn existing_replay_incoming(
    existing: &Constellation,
    row: &ExistingBatchReplayRow,
) -> Constellation {
    let mut incoming = existing.clone();
    incoming.anchors = row.anchors.clone();
    incoming.flags.ungrounded = incoming.anchors.is_empty();
    incoming
}

pub(crate) fn should_stage_batch_constellation(new: bool, marker_kinds: &[AnchorKind]) -> bool {
    new || !marker_kinds.is_empty()
}

pub(crate) fn ensure_idempotent_batch_replay(
    vault: &AsterVault,
    cx: &calyx_core::Constellation,
) -> CliResult<calyx_core::Constellation> {
    let existing = vault.get(cx.cx_id, vault.snapshot())?;
    if existing.panel_version != cx.panel_version
        || existing.input_ref != cx.input_ref
        || existing.modality != cx.modality
        || existing.metadata != cx.metadata
    {
        return Err(CliError::usage(format!(
            "idempotent batch replay for cx {} changed stored non-anchor identity: {}",
            cx.cx_id,
            identity_mismatch_reason(
                IdentityFields {
                    panel_version: existing.panel_version,
                    input_ref: &existing.input_ref,
                    modality: existing.modality,
                    metadata: &existing.metadata,
                },
                IdentityFields {
                    panel_version: cx.panel_version,
                    input_ref: &cx.input_ref,
                    modality: cx.modality,
                    metadata: &cx.metadata,
                },
            )
        )));
    }
    Ok(existing)
}

pub(crate) fn identity_mismatch_reason(
    existing: IdentityFields<'_>,
    incoming: IdentityFields<'_>,
) -> String {
    let mut reasons = Vec::new();
    if existing.panel_version != incoming.panel_version {
        reasons.push(format!(
            "panel_version existing={} incoming={}",
            existing.panel_version, incoming.panel_version
        ));
    }
    if existing.input_ref != incoming.input_ref {
        let mut input_parts = Vec::new();
        if existing.input_ref.hash != incoming.input_ref.hash {
            input_parts.push("hash");
        }
        if existing.input_ref.pointer != incoming.input_ref.pointer {
            input_parts.push("pointer");
        }
        if existing.input_ref.redacted != incoming.input_ref.redacted {
            input_parts.push("redacted");
        }
        reasons.push(format!("input_ref fields={}", input_parts.join(",")));
    }
    if existing.modality != incoming.modality {
        reasons.push(format!(
            "modality existing={:?} incoming={:?}",
            existing.modality, incoming.modality
        ));
    }
    if existing.metadata != incoming.metadata {
        let existing_keys = existing.metadata.keys().cloned().collect::<BTreeSet<_>>();
        let incoming_keys = incoming.metadata.keys().cloned().collect::<BTreeSet<_>>();
        let removed = existing_keys
            .difference(&incoming_keys)
            .take(8)
            .cloned()
            .collect::<Vec<_>>();
        let added = incoming_keys
            .difference(&existing_keys)
            .take(8)
            .cloned()
            .collect::<Vec<_>>();
        let changed = existing_keys
            .intersection(&incoming_keys)
            .filter(|key| existing.metadata.get(*key) != incoming.metadata.get(*key))
            .take(8)
            .cloned()
            .collect::<Vec<_>>();
        reasons.push(format!(
            "metadata removed_keys={removed:?} added_keys={added:?} changed_keys={changed:?}"
        ));
    }
    if reasons.is_empty() {
        "unknown identity mismatch".to_string()
    } else {
        reasons.join("; ")
    }
}

pub(crate) fn append_missing_batch_anchors(
    vault: &AsterVault,
    existing: &calyx_core::Constellation,
    incoming: &calyx_core::Constellation,
    marker_kinds: &[AnchorKind],
) -> CliResult<calyx_core::Constellation> {
    if let AnchorConflictResult::Conflicting {
        anchor_type,
        reason,
    } = check_anchor_conflict(incoming, existing)
    {
        return Err(calyx_core::CalyxError::aster_corrupt_shard(format!(
            "idempotent batch replay for cx {} has conflicting {anchor_type:?} anchor: {reason:?}",
            incoming.cx_id
        ))
        .into());
    }
    if marker_kinds.is_empty() {
        return Ok(existing.clone());
    }

    let marker_kinds = marker_kinds.iter().collect::<BTreeSet<_>>();
    let mut merged = existing.clone();
    let mut added = Vec::new();
    for anchor in &incoming.anchors {
        if marker_kinds.contains(&anchor.kind) {
            merged.anchors.push(anchor.clone());
            added.push(anchor.clone());
        }
    }
    if added.is_empty() {
        return Ok(existing.clone());
    }
    merged.flags.ungrounded = merged.anchors.is_empty();
    merged.validate_schema()?;

    let mut rows = Vec::with_capacity(1 + added.len());
    rows.push((
        ColumnFamily::Base,
        base_key(incoming.cx_id),
        encode::encode_constellation_base(&merged)?,
    ));
    for anchor in added {
        rows.push((
            ColumnFamily::Anchors,
            anchor_key(incoming.cx_id, &anchor.kind),
            encode::encode_anchor(&anchor)?,
        ));
    }
    vault.write_cf_batch(rows)?;
    Ok(merged)
}

pub(crate) struct BatchOrderRow {
    pub(crate) cx_id: CxId,
    pub(crate) expected_readback: calyx_core::Constellation,
    pub(crate) new: bool,
    pub(crate) marker_kinds: Vec<AnchorKind>,
    pub(crate) oracle: Option<OracleEvent>,
}

pub(crate) fn append_oracle_events(vault: &AsterVault, order: &[BatchOrderRow]) -> CliResult<()> {
    for row in order {
        if let Some(event) = &row.oracle {
            append_recurrence_if_absent(vault, row.cx_id, event, now_ms())?;
        }
    }
    Ok(())
}

pub(crate) fn current_anchor_kinds(
    vault: &AsterVault,
    cx_id: CxId,
    exists: bool,
) -> CliResult<BTreeSet<AnchorKind>> {
    if !exists {
        return Ok(BTreeSet::new());
    }
    Ok(vault
        .get(cx_id, vault.snapshot())?
        .anchors
        .into_iter()
        .map(|anchor| anchor.kind)
        .collect())
}
