mod engine;
mod filters;
mod output;
mod parse;
mod persisted;

pub(crate) use parse::{KernelAnswerArgs, SearchArgs};
#[cfg(test)]
pub(crate) use parse::{SearchFreshnessArg, SearchFusionArg, SearchGuardArg};
pub(crate) use persisted::{PersistedSearchIndexes, load_docs};

use super::Subcommand;
use crate::error::CliResult;
use calyx_aster::vault::AsterVault;
use std::path::Path;

pub(crate) fn run(command: Subcommand) -> CliResult {
    engine::run(command)
}

pub(crate) fn rebuild_persistent_indexes(vault_dir: &Path, vault: &AsterVault) -> CliResult {
    persisted::rebuild_for_vault(vault_dir, vault)
}

pub(crate) fn measure_text_query_vectors(
    state: &calyx_registry::VaultPanelState,
    query: &str,
) -> CliResult<Vec<(calyx_core::SlotId, calyx_core::SlotVector)>> {
    engine::measure_query_vectors(state, query)
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
