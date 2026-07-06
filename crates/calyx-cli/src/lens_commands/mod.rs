mod card;
pub(crate) mod catalog;
mod commission;
mod explain;
mod flags;
mod remove;
mod scale_audit;
pub(crate) mod support;

#[cfg(test)]
mod tests;

use crate::error::{CliError, CliResult};

pub(crate) fn run(topic: &str, rest: &[String]) -> CliResult {
    match topic {
        "add" => catalog::add(rest),
        "card" => card::card(rest),
        "list" => catalog::list(rest),
        "migrate-catalog" => catalog::migrate_catalog(rest),
        "remove" => remove::remove(rest),
        "explain" => explain::explain(rest),
        "commission" => commission::commission(rest),
        "scale-audit" => scale_audit::scale_audit(rest),
        other => Err(CliError::usage(format!(
            "unknown lens subcommand {other}; expected add, list, migrate-catalog, remove, card, explain, commission, or scale-audit"
        ))),
    }
}
