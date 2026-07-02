mod engine;
mod output;
mod parse;

pub(crate) use calyx_search::{PersistedSearchIndexes, load_docs};
pub(crate) use parse::{KernelAnswerArgs, SearchArgs, parse_resident_addr};
#[cfg(test)]
pub(crate) use parse::{SearchFreshnessArg, SearchFusionArg, SearchGuardArg};

use super::vault::{home_dir, resolve_vault_info, vault_salt};
use super::{Subcommand, VaultRefArgs};
use crate::bounded_progress::ProgressSink;
use crate::error::{CliError, CliResult};
use calyx_aster::cf::ColumnFamily;
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_registry::{load_vault_panel_state, require_vault_registry_contracts};
use serde_json::json;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub(crate) fn run(command: Subcommand) -> CliResult {
    match command {
        Subcommand::RebuildSearchIndex(args) => run_rebuild_search_index(args),
        other => engine::run(other),
    }
}

/// Rebuild the persistent search-index sidecars for an existing vault, without
/// re-ingesting. Recovers a vault whose ingest-time index rebuild was interrupted
/// (and gives a standalone way to refresh sidecars after the fixed serialization).
fn run_rebuild_search_index(args: VaultRefArgs) -> CliResult {
    calyx_search::validate_rebuild_config()?;
    let resolved = resolve_vault_info(&home_dir()?, &args.vault)?;
    let state = load_vault_panel_state(&resolved.path)?;
    let vault = AsterVault::open(
        &resolved.path,
        resolved.vault_id,
        vault_salt(resolved.vault_id, &resolved.name),
        latest_read_vault_options_for_cfs(panel_read_cfs(&state.panel)),
    )?;
    require_vault_registry_contracts(&resolved.path)?;
    let progress_path = rebuild_progress_path(&resolved.path)?;
    let progress_arg = progress_path.to_string_lossy().to_string();
    let mut progress = ProgressSink::from_arg(Some(&progress_arg))?;
    emit_rebuild_progress_record(
        &mut progress,
        json!({
            "phase": "run_start",
            "vault": resolved.name,
            "vault_dir": resolved.path.display().to_string(),
            "progress_artifact": progress_path.display().to_string(),
        }),
    )?;
    let rebuild =
        calyx_search::rebuild_for_vault_with_fallible_progress(&resolved.path, &vault, |event| {
            emit_rebuild_progress(&mut progress, event).map_err(search_progress_error)
        });
    if let Err(error) = rebuild {
        let cli_error = CliError::from(error);
        let _ = emit_rebuild_progress_record(
            &mut progress,
            json!({
                "phase": "failed",
                "error": {
                    "code": cli_error.code(),
                    "message": cli_error.message(),
                    "remediation": cli_error.remediation(),
                },
            }),
        );
        return Err(cli_error);
    }
    emit_rebuild_progress_record(&mut progress, json!({"phase": "complete"}))?;
    // Physical readback: a completed rebuild must have cleared the durable
    // rebuild-required marker; a survivor means the clear was lost and the
    // vault's crash-recovery state cannot be trusted.
    if let Some(marker) = calyx_search::read_rebuild_required_marker(&resolved.path)? {
        return Err(CliError::from(calyx_core::CalyxError {
            code: "CALYX_SEARCH_REBUILD_MARKER_STUCK",
            message: format!(
                "rebuild completed but the rebuild-required marker at {} still exists (source={}, required_base_seq={:?})",
                calyx_search::rebuild_required_marker_path(&resolved.path).display(),
                marker.source,
                marker.required_base_seq
            ),
            remediation: "rerun `calyx rebuild-search-index <vault>` and inspect filesystem health if the marker persists",
        }));
    }
    crate::output::print_json(&serde_json::json!({
        "status": "ok",
        "vault": resolved.name,
        "vault_dir": resolved.path.display().to_string(),
        "progress_artifact": progress_path.display().to_string(),
        "rebuild_required_marker": serde_json::Value::Null,
    }))
}

pub(crate) fn rebuild_persistent_indexes(vault_dir: &Path, vault: &AsterVault) -> CliResult {
    Ok(calyx_search::rebuild_for_vault(vault_dir, vault)?)
}

fn rebuild_progress_path(vault_dir: &Path) -> CliResult<PathBuf> {
    let root = vault_dir.join("idx").join("search");
    fs::create_dir_all(&root).map_err(CliError::from)?;
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| CliError::io(format!("system clock before UNIX_EPOCH: {error}")))?
        .as_millis();
    Ok(root.join(format!(
        "rebuild-progress-{millis}-{}.jsonl",
        std::process::id()
    )))
}

fn search_progress_error(error: CliError) -> calyx_search::SearchError {
    calyx_search::SearchError::Io(format!(
        "write rebuild progress artifact failed: code={} message={} remediation={}",
        error.code(),
        error.message(),
        error.remediation()
    ))
}

fn emit_rebuild_progress(
    progress: &mut ProgressSink,
    event: calyx_search::RebuildProgress<'_>,
) -> CliResult {
    emit_rebuild_progress_record(
        progress,
        json!({
            "phase": event.phase,
            "slot": event.slot.map(|slot| slot.get()),
            "rows": event.rows,
            "base_seq": event.base_seq,
            "manifest_path": event.manifest_path.map(|path| path.display().to_string()),
            "detail": event.detail,
        }),
    )
}

fn emit_rebuild_progress_record(
    progress: &mut ProgressSink,
    mut value: serde_json::Value,
) -> CliResult {
    let object = value
        .as_object_mut()
        .ok_or_else(|| CliError::usage("search rebuild progress record must be a JSON object"))?;
    object.insert(
        "schema".to_string(),
        json!("calyx-search-rebuild-progress-v1"),
    );
    object.insert("event".to_string(), json!("search_rebuild.progress"));
    progress.emit(value)
}

pub(crate) fn rebuild_persistent_indexes_with_progress<F>(
    vault_dir: &Path,
    vault: &AsterVault,
    progress: F,
) -> CliResult
where
    F: FnMut(calyx_search::RebuildProgress<'_>) + Send,
{
    Ok(calyx_search::rebuild_for_vault_with_progress(
        vault_dir, vault, progress,
    )?)
}

pub(super) fn latest_read_vault_options_for_cfs(
    selected_cfs: Option<Vec<ColumnFamily>>,
) -> VaultOptions {
    VaultOptions {
        restore_mvcc_rows: false,
        restore_ledger_hook: false,
        read_only: true,
        selected_cfs,
        ..VaultOptions::default()
    }
}

pub(super) fn base_read_cfs() -> Vec<ColumnFamily> {
    vec![ColumnFamily::Base]
}

pub(super) fn panel_read_cfs(panel: &calyx_core::Panel) -> Option<Vec<ColumnFamily>> {
    let mut cfs = vec![ColumnFamily::Base];
    cfs.extend(
        panel
            .slots
            .iter()
            .map(|slot| ColumnFamily::slot(slot.slot_id)),
    );
    cfs.sort();
    cfs.dedup();
    Some(cfs)
}

pub(crate) fn measure_text_query_vectors(
    state: &calyx_registry::VaultPanelState,
    query: &str,
) -> CliResult<Vec<(calyx_core::SlotId, calyx_core::SlotVector)>> {
    Ok(calyx_search::measure_query_vectors(state, query)?)
}

pub(crate) fn parse_search(rest: &[String]) -> CliResult<Subcommand> {
    parse::parse_search(rest)
}

pub(crate) fn parse_kernel_answer(rest: &[String]) -> CliResult<Subcommand> {
    parse::parse_kernel_answer(rest)
}

#[cfg(test)]
pub(crate) use parse::{kernel_answer_tokens, search_tokens};

#[cfg(test)]
mod tests;
