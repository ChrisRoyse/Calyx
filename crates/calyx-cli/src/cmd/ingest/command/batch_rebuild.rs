//! Post-commit derived-index rebuild for batch ingest (issue #1089).
//!
//! Runs after the Base CF rows are durable and physically read back. The
//! rebuild-required marker is prepared and bound to the exact durable commit
//! sequence under Aster's commit lock. The rebuild itself resumes staged slot
//! artifacts and clears the marker only behind the same derived-state fence.

use super::*;

pub(super) fn run_post_commit_index_rebuild(
    resolved: &ResolvedVault,
    vault: &AsterVault,
    state: &VaultPanelState,
    summary: &BatchIngestSummary,
    committed_content_seq: Option<u64>,
    session: &mut Option<&mut BatchIngestSession>,
) -> CliResult<()> {
    if summary.new_count > 0 && committed_content_seq.is_none() {
        return Err(calyx_core::CalyxError {
            code: "CALYX_SEARCH_REBUILD_MARKER_INTENT_INVALID",
            message: format!(
                "batch summary reports {} new constellations but no Aster-committed rebuild intent sequence",
                summary.new_count
            ),
            remediation: "preserve the vault, inspect the Base/WAL rows and rebuild-required marker, then reconcile the ingest commit before rebuilding search",
        }
        .into());
    }
    if let Some(required_seq) = committed_content_seq {
        if let Some(session) = session.as_deref_mut() {
            session.record_index_phase("running")?;
        }
        ingest_runtime_log(format_args!(
            "phase=batch_index_rebuild_start new_count={} already_count={} required_base_seq={required_seq}",
            summary.new_count, summary.already_count,
        ));
        let rebuild = if let Some(session) = session.as_deref_mut() {
            rebuild_persistent_indexes_with_fallible_progress(
                &resolved.path,
                vault,
                state,
                |event| {
                    log_batch_index_rebuild_progress(event);
                    session.check_interrupted()
                },
            )
        } else {
            rebuild_persistent_indexes_with_fallible_progress(
                &resolved.path,
                vault,
                state,
                |event| {
                    log_batch_index_rebuild_progress(event);
                    Ok(())
                },
            )
        };
        if let Err(error) = rebuild {
            log_batch_index_rebuild_error(summary, &error);
            if error.code() == "CALYX_INGEST_SESSION_INTERRUPTED" {
                return Err(error);
            }
            return Err(batch_index_rebuild_error(resolved, summary, error));
        }
        ingest_runtime_log(format_args!(
            "phase=batch_index_rebuild_ok new_count={} already_count={}",
            summary.new_count, summary.already_count
        ));
        if let Some(session) = session.as_deref_mut() {
            session.record_index_phase("complete")?;
        }
        return Ok(());
    }
    ingest_runtime_log(format_args!(
        "phase=batch_index_rebuild_skip reason=no_new_constellations already_count={} latest_seq={} derived_content_seq={}",
        summary.already_count,
        vault.latest_seq(),
        vault.derived_content_seq()
    ));
    if let Some(session) = session.as_deref_mut() {
        session.record_index_phase("skipped")?;
    }
    let existing_marker = calyx_search::read_rebuild_required_marker(&resolved.path)?;
    ingest_runtime_log(format_args!(
        "phase=derived_rebuild_marker_unchanged reason=no_content_commit marker_present={}",
        existing_marker.is_some()
    ));
    Ok(())
}

fn log_batch_index_rebuild_progress(event: calyx_search::RebuildProgress<'_>) {
    let slot = event
        .slot
        .map(|slot| slot.get().to_string())
        .unwrap_or_else(|| "-".to_string());
    let rows = event
        .rows
        .map(|rows| rows.to_string())
        .unwrap_or_else(|| "-".to_string());
    let base_seq = event
        .base_seq
        .map(|seq| seq.to_string())
        .unwrap_or_else(|| "-".to_string());
    let manifest_path = event
        .manifest_path
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "-".to_string());
    let detail = event
        .detail
        .as_deref()
        .map(json_string)
        .unwrap_or_else(|| "-".to_string());
    ingest_runtime_log(format_args!(
        "phase=batch_index_rebuild_{} slot={} rows={} base_seq={} manifest_path={} detail={}",
        event.phase, slot, rows, base_seq, manifest_path, detail
    ));
}

fn log_batch_index_rebuild_error(summary: &BatchIngestSummary, error: &CliError) {
    ingest_runtime_log(format_args!(
        "phase=batch_index_rebuild_error error_code={} error_message_json={} row_count={} new_count={} already_count={} verified_base_rows={}",
        error.code(),
        json_string(error.message()),
        summary.row_count,
        summary.new_count,
        summary.already_count,
        summary.verified_base_rows
    ));
}

fn batch_index_rebuild_error(
    resolved: &ResolvedVault,
    summary: &BatchIngestSummary,
    error: CliError,
) -> CliError {
    CliError::from(calyx_core::CalyxError {
        code: "CALYX_INGEST_INDEX_REBUILD_FAILED",
        message: format!(
            "batch ingest committed and verified {} Base CF rows in vault {}; post-commit persistent search-index rebuild failed after summary emission point (row_count={}, new_count={}, already_count={}, first_cx_id={}, last_cx_id={}, rebuild_required_marker={}, cause_code={}, cause_message={})",
            summary.verified_base_rows,
            resolved.path.display(),
            summary.row_count,
            summary.new_count,
            summary.already_count,
            summary.first_cx_id.as_deref().unwrap_or("<none>"),
            summary.last_cx_id.as_deref().unwrap_or("<none>"),
            calyx_search::rebuild_required_marker_path(&resolved.path).display(),
            error.code(),
            error.message()
        ),
        remediation: "inspect CALYX_INGEST_RUNTIME phase=batch_index_rebuild_error and the rebuild-required marker, fix the named search-index or vault state issue, then run `calyx rebuild-search-index <vault>` (resumes staged slot artifacts) before search",
    })
}
