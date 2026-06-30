mod engine;
mod output;
mod parse;

pub(crate) use calyx_search::{PersistedSearchIndexes, load_docs};
pub(crate) use parse::{KernelAnswerArgs, SearchArgs};
#[cfg(test)]
pub(crate) use parse::{SearchFreshnessArg, SearchFusionArg, SearchGuardArg};

use super::vault::{home_dir, resolve_vault_info, vault_salt};
use super::{Subcommand, VaultRefArgs};
use crate::error::CliResult;
use calyx_aster::cf::ColumnFamily;
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_registry::{load_vault_panel_state, require_vault_registry_contracts};
use std::path::Path;

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
    let resolved = resolve_vault_info(&home_dir()?, &args.vault)?;
    let state = load_vault_panel_state(&resolved.path)?;
    let vault = AsterVault::open(
        &resolved.path,
        resolved.vault_id,
        vault_salt(resolved.vault_id, &resolved.name),
        latest_read_vault_options_for_cfs(panel_read_cfs(&state.panel)),
    )?;
    require_vault_registry_contracts(&resolved.path)?;
    rebuild_persistent_indexes(&resolved.path, &vault)?;
    crate::output::print_json(&serde_json::json!({
        "status": "ok",
        "vault": resolved.name,
        "vault_dir": resolved.path.display().to_string(),
    }))
}

pub(crate) fn rebuild_persistent_indexes(vault_dir: &Path, vault: &AsterVault) -> CliResult {
    Ok(calyx_search::rebuild_for_vault(vault_dir, vault)?)
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
