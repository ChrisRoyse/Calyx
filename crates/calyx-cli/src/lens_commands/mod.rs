mod card;
mod catalog;
mod commission;
mod explain;
mod flags;
pub(crate) mod support;

#[cfg(test)]
mod tests;

use crate::error::{CliError, CliResult};

pub(crate) fn run(topic: &str, rest: &[String]) -> CliResult {
    match topic {
        "add" => catalog::add(rest),
        "card" => card::card(rest),
        "list" => catalog::list(rest),
        "explain" => explain::explain(rest),
        "commission" => commission::commission(rest),
        other => Err(CliError::usage(format!(
            "unknown lens subcommand {other}; expected add, list, card, explain, or commission"
        ))),
    }
}
