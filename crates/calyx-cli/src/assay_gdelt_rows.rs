//! Convert real GDELT v2 event exports into timestamped assay rows.

mod args;
mod convert;
mod report;

#[cfg(test)]
mod tests;

use calyx_core::CalyxError;

use crate::error::{CliError, CliResult};
use crate::output::print_json;

pub(crate) fn run(raw: &[String]) -> CliResult {
    let args = args::Args::parse(raw)?;
    let report = convert::run(&args)?;
    print_json(&report)
}

pub(crate) fn local_error(
    code: &'static str,
    message: impl Into<String>,
    remediation: &'static str,
) -> CliError {
    CliError::Calyx(CalyxError {
        code,
        message: message.into(),
        remediation,
    })
}
