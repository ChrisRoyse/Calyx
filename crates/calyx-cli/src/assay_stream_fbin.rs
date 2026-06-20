//! Stream real corpus rows through frozen lenses directly into per-slot vector files.

mod args;
mod format;
mod rows;
#[cfg(test)]
mod tests;
mod write;

use calyx_core::CalyxError;

use crate::error::{CliError, CliResult};
use crate::output::print_json;

pub(crate) const MIN_A35_LENSES: usize = 10;
pub(crate) const DEFAULT_MIN_BITS: f32 = 0.05;

pub(crate) fn run(raw: &[String]) -> CliResult {
    let args = args::Args::parse(raw)?;
    if args.worker_report.is_some() {
        let evidence = write::run_worker(&args)?;
        return print_json(&evidence);
    }
    let evidence = write::run(&args)?;
    print_json(&evidence)
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

pub(crate) fn io_error(error: std::io::Error) -> CliError {
    CliError::io(error.to_string())
}
