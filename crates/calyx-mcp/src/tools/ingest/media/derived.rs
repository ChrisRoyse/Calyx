use super::*;
use crate::tools::ingest::IngestReport;

pub(super) fn ingest_media_with_derived_text(
    resolved: &ResolvedVault,
    retained: RetainedMediaInput,
) -> ToolResult<Vec<IngestReport>> {
    let vault = open_vault(resolved)?;
    let state = calyx_registry::load_vault_panel_state(&resolved.path)?;
    ensure_raw_media_panel_route(retained.input.modality, &state)?;
    let source_cx_id = vault.cx_id_for_input(&retained.input.bytes, state.panel.version);
    let derived = derived_text::derive_text_for_media(&resolved.path, &retained, source_cx_id)?;

    let mut media =
        measure_constellation(&vault, &state, retained.input.clone(), now_ms())?.constellation;
    media.metadata = retained.metadata.clone();
    let mut text =
        measure_constellation(&vault, &state, derived.input.clone(), now_ms())?.constellation;
    text.metadata = derived.metadata.clone();

    let media_new = !base_exists(&vault, media.cx_id)?;
    let text_new = !base_exists(&vault, text.cx_id)?;
    let payload =
        derived_text::derivation_ledger_payload(&retained, &derived, media.cx_id, text.cx_id)?;
    let mut staged = Vec::with_capacity(2);
    if media_new {
        staged.push(media.clone());
    }
    if text_new && text.cx_id != media.cx_id {
        staged.push(text.clone());
    }
    let artifact_draft =
        derived_text::derived_artifact_draft(&retained, &derived, media.cx_id, text.cx_id)?;
    let commit = vault.put_batch_with_ingest_ledger_and_media_artifact(
        staged,
        SubjectId::Cx(text.cx_id),
        payload,
        ActorId::Service("calyx-mcp".to_string()),
        artifact_draft,
    )?;
    vault.flush()?;
    let snapshot = vault.snapshot();
    verify_media_readback(&vault, snapshot, &media, media_new)?;
    verify_media_readback(&vault, snapshot, &text, text_new)?;
    verify_media_artifact_readback(&vault, snapshot, &commit.artifact)?;

    let media_seq = if media_new {
        vault.get(media.cx_id, snapshot)?.provenance.seq
    } else {
        commit.artifact.ledger_ref.seq
    };
    let text_seq = if text_new {
        vault.get(text.cx_id, snapshot)?.provenance.seq
    } else {
        commit.artifact.ledger_ref.seq
    };
    vault.flush()?;
    Ok(vec![
        IngestReport {
            cx_id: media.cx_id.to_string(),
            new: media_new,
            ledger_seq: media_seq,
        },
        IngestReport {
            cx_id: text.cx_id.to_string(),
            new: text_new,
            ledger_seq: text_seq,
        },
    ])
}

fn ensure_raw_media_panel_route(
    modality: Modality,
    state: &calyx_registry::VaultPanelState,
) -> ToolResult<()> {
    if !matches!(
        modality,
        Modality::Image | Modality::Audio | Modality::Video
    ) {
        return Ok(());
    }
    let has_declared_route = state
        .panel
        .slots
        .iter()
        .any(|slot| slot.state == SlotState::Active && slot.counts_toward_degraded(modality));
    if has_declared_route {
        return Ok(());
    }
    Err(CalyxError {
        code: "CALYX_MEDIA_ROUTE_UNAVAILABLE",
        message: format!(
            "raw {modality:?} ingest requires an active {modality:?} content lens before derived text can be attached"
        ),
        remediation:
            "add or activate an image/audio/video lens for the raw media modality, then re-run ingest so the media constellation is measured instead of empty",
    }
    .into())
}

fn verify_media_readback(
    vault: &calyx_aster::vault::AsterVault,
    snapshot: u64,
    expected: &calyx_core::Constellation,
    new: bool,
) -> ToolResult<()> {
    let stored = vault.get(expected.cx_id, snapshot)?;
    let mismatch = if new {
        stored.panel_version != expected.panel_version
            || stored.input_ref != expected.input_ref
            || stored.modality != expected.modality
            || stored.slots != expected.slots
            || stored.metadata != expected.metadata
            || stored.flags != expected.flags
    } else {
        stored.panel_version != expected.panel_version
            || stored.input_ref.hash != expected.input_ref.hash
            || stored.modality != expected.modality
            || stored.slots != expected.slots
    };
    if mismatch {
        return Err(CalyxError::aster_corrupt_shard(format!(
            "durable MCP media ingest readback mismatch for cx {}",
            expected.cx_id
        ))
        .into());
    }
    Ok(())
}

fn verify_media_artifact_readback(
    vault: &calyx_aster::vault::AsterVault,
    snapshot: u64,
    expected: &calyx_aster::media_artifact::DerivedMediaArtifactRecord,
) -> ToolResult<()> {
    let stored = vault
        .get_derived_media_artifact(snapshot, &expected.artifact_id)?
        .ok_or_else(|| {
            CalyxError::aster_corrupt_shard(format!(
                "derived media artifact {} missing after commit",
                expected.artifact_id
            ))
        })?;
    if stored != *expected {
        return Err(CalyxError::aster_corrupt_shard(format!(
            "derived media artifact {} readback mismatch",
            expected.artifact_id
        ))
        .into());
    }
    let source_records =
        vault.derived_media_artifacts_for_source(snapshot, expected.source_cx_id)?;
    if !source_records.iter().any(|record| record == expected) {
        return Err(CalyxError::aster_corrupt_shard(format!(
            "derived media artifact {} missing from source index",
            expected.artifact_id
        ))
        .into());
    }
    let target_records =
        vault.derived_media_artifacts_for_target(snapshot, expected.target_cx_id)?;
    if !target_records.iter().any(|record| record == expected) {
        return Err(CalyxError::aster_corrupt_shard(format!(
            "derived media artifact {} missing from target index",
            expected.artifact_id
        ))
        .into());
    }
    Ok(())
}
