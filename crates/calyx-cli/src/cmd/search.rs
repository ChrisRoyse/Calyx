mod engine;
mod filters;
mod output;
mod parse;

pub(crate) use parse::{KernelAnswerArgs, SearchArgs};
#[cfg(test)]
pub(crate) use parse::{SearchFreshnessArg, SearchFusionArg, SearchGuardArg};

use super::Subcommand;
use crate::error::CliResult;

pub(crate) fn run(command: Subcommand) -> CliResult {
    engine::run(command)
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
