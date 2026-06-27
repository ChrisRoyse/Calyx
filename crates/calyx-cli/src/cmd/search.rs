mod engine;
mod output;
mod parse;

pub(crate) use calyx_search::{PersistedSearchIndexes, load_docs};
pub(crate) use parse::{KernelAnswerArgs, SearchArgs};
#[cfg(test)]
pub(crate) use parse::{SearchFreshnessArg, SearchFusionArg, SearchGuardArg};

use super::Subcommand;
use crate::error::CliResult;
use calyx_aster::vault::AsterVault;
use std::path::Path;

pub(crate) fn run(command: Subcommand) -> CliResult {
    engine::run(command)
}

pub(crate) fn rebuild_persistent_indexes(vault_dir: &Path, vault: &AsterVault) -> CliResult {
    Ok(calyx_search::rebuild_for_vault(vault_dir, vault)?)
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
