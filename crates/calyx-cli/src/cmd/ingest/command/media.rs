use std::net::SocketAddr;

use calyx_aster::vault::AsterVault;
use calyx_core::{Modality, SlotState, VaultStore};
use calyx_ledger::{ActorId, SubjectId};
use calyx_registry::{VaultPanelState, load_vault_panel_state};

use super::ingest_runtime_log;
use crate::cmd::ingest::constellation::{
    ensure_content_panel_floor, measure_constellation_with_runtime_limit,
};
use crate::cmd::ingest::store::{base_exists, open_vault};
use crate::cmd::ingest::types::IngestReport;
use crate::cmd::ingest::verify::verify_base_readback;
use crate::cmd::search::rebuild_persistent_indexes;
use crate::cmd::vault::{ResolvedVault, now_ms};
use crate::error::CliResult;
use crate::media_derived_text::{
    derivation_ledger_payload, derive_text_for_media, derived_artifact_draft,
};
use crate::raw_media::{RetainedMediaInput, media_metadata};

pub(super) fn ingest_media_with_derived_text(
    resolved: &ResolvedVault,
    retained: RetainedMediaInput,
    resident_addr: Option<SocketAddr>,
) -> CliResult<Vec<IngestReport>> {
    let vault = open_vault(resolved)?;
    ingest_runtime_log(format_args!(
        "phase=load_vault_panel_state_start vault={}",
        resolved.path.display()
    ));
    let state = load_vault_panel_state(&resolved.path)?;
    ingest_runtime_log(format_args!(
        "phase=load_vault_panel_state_ok vault={} panel_version={} slots={}",
        resolved.path.display(),
        state.panel.version,
        state.panel.slots.len()
    ));

    ensure_raw_media_panel_route(retained.input.modality, &state)?;
    let source_cx_id = vault.cx_id_for_input(&retained.input.bytes, state.panel.version);
    let derived = derive_text_for_media(&resolved.path, &retained, source_cx_id)?;

    let mut media_cx = measure_constellation_with_runtime_limit(
        &vault,
        &state,
        &retained.input,
        now_ms(),
        None,
        resident_addr,
    )?;
    media_cx.metadata = media_metadata(&retained);
    ensure_content_panel_floor(&media_cx, &state)?;
    let mut text_cx = measure_constellation_with_runtime_limit(
        &vault,
        &state,
        &derived.input,
        now_ms(),
        None,
        resident_addr,
    )?;
    text_cx.metadata = derived.metadata.clone();
    ensure_content_panel_floor(&text_cx, &state)?;

    let media_new = !base_exists(&vault, media_cx.cx_id)?;
    let text_new = !base_exists(&vault, text_cx.cx_id)?;
    let payload = derivation_ledger_payload(&retained, &derived, media_cx.cx_id, text_cx.cx_id)?;
    let mut staged = Vec::with_capacity(2);
    if media_new {
        staged.push(media_cx.clone());
    }
    if text_new && text_cx.cx_id != media_cx.cx_id {
        staged.push(text_cx.clone());
    }
    let artifact_draft =
        derived_artifact_draft(&retained, &derived, media_cx.cx_id, text_cx.cx_id)?;
    super::stake_rebuild_required_marker(
        &resolved.path,
        "media_ingest",
        format!(
            "media ingest of {:?} input with derived text (media cx {}, text cx {})",
            retained.input.modality, media_cx.cx_id, text_cx.cx_id
        ),
        None,
        None,
    )?;
    let commit = vault.put_batch_with_ingest_ledger_and_media_artifact(
        staged,
        SubjectId::Cx(text_cx.cx_id),
        payload,
        ActorId::Service("calyx-cli".to_string()),
        artifact_draft,
    )?;
    vault.flush()?;
    rebuild_persistent_indexes(&resolved.path, &vault)?;

    let snapshot = vault.snapshot();
    if media_new {
        verify_base_readback(&vault, snapshot, &media_cx, media_cx.cx_id, &[])?;
    } else {
        verify_existing_media_or_text_readback(&vault, snapshot, &media_cx)?;
    }
    if text_new {
        verify_base_readback(&vault, snapshot, &text_cx, text_cx.cx_id, &[])?;
    } else {
        verify_existing_media_or_text_readback(&vault, snapshot, &text_cx)?;
    }
    verify_media_artifact_readback(&vault, snapshot, &commit.artifact)?;

    let media_ledger_seq = if media_new {
        vault.get(media_cx.cx_id, snapshot)?.provenance.seq
    } else {
        commit.artifact.ledger_ref.seq
    };
    let text_ledger_seq = if text_new {
        vault.get(text_cx.cx_id, snapshot)?.provenance.seq
    } else {
        commit.artifact.ledger_ref.seq
    };
    vault.flush()?;
    Ok(vec![
        IngestReport {
            cx_id: media_cx.cx_id.to_string(),
            new: media_new,
            ledger_seq: media_ledger_seq,
        },
        IngestReport {
            cx_id: text_cx.cx_id.to_string(),
            new: text_new,
            ledger_seq: text_ledger_seq,
        },
    ])
}

fn ensure_raw_media_panel_route(modality: Modality, state: &VaultPanelState) -> CliResult {
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
    Err(calyx_core::CalyxError {
        code: "CALYX_MEDIA_ROUTE_UNAVAILABLE",
        message: format!(
            "raw {modality:?} ingest requires an active {modality:?} content lens before derived text can be attached"
        ),
        remediation:
            "add or activate an image/audio/video lens for the raw media modality, then re-run ingest so the media constellation is measured instead of empty",
    }
    .into())
}

fn verify_existing_media_or_text_readback(
    vault: &AsterVault,
    snapshot: u64,
    expected: &calyx_core::Constellation,
) -> CliResult {
    let stored = vault.get(expected.cx_id, snapshot)?;
    if stored.panel_version != expected.panel_version
        || stored.input_ref.hash != expected.input_ref.hash
        || stored.modality != expected.modality
        || stored.slots != expected.slots
    {
        return Err(calyx_core::CalyxError::aster_corrupt_shard(format!(
            "durable media ingest readback mismatch for existing cx {}",
            expected.cx_id
        ))
        .into());
    }
    Ok(())
}

fn verify_media_artifact_readback(
    vault: &AsterVault,
    snapshot: u64,
    expected: &calyx_aster::media_artifact::DerivedMediaArtifactRecord,
) -> CliResult {
    let stored = vault
        .get_derived_media_artifact(snapshot, &expected.artifact_id)?
        .ok_or_else(|| {
            calyx_core::CalyxError::aster_corrupt_shard(format!(
                "derived media artifact {} missing after commit",
                expected.artifact_id
            ))
        })?;
    if stored != *expected {
        return Err(calyx_core::CalyxError::aster_corrupt_shard(format!(
            "derived media artifact {} readback mismatch",
            expected.artifact_id
        ))
        .into());
    }
    let source_records =
        vault.derived_media_artifacts_for_source(snapshot, expected.source_cx_id)?;
    if !source_records.iter().any(|record| record == expected) {
        return Err(calyx_core::CalyxError::aster_corrupt_shard(format!(
            "derived media artifact {} missing from source index",
            expected.artifact_id
        ))
        .into());
    }
    let target_records =
        vault.derived_media_artifacts_for_target(snapshot, expected.target_cx_id)?;
    if !target_records.iter().any(|record| record == expected) {
        return Err(calyx_core::CalyxError::aster_corrupt_shard(format!(
            "derived media artifact {} missing from target index",
            expected.artifact_id
        ))
        .into());
    }
    Ok(())
}
