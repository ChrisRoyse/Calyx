use std::{env, process::ExitCode};

use crate::error::CliError;

pub(crate) fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    if let Some(code) = super::verify_restore::try_run(&args) {
        return code;
    }
    // `healthcheck --config <toml>` is the daemon-readiness probe (PH65 T04),
    // which owns its 0/1/2 exit contract; the plain `healthcheck` deploy-health
    // command falls through to the generic dispatcher below.
    if let Some(code) = super::healthcheck_daemon::try_run(&args) {
        return code;
    }
    match crate::dispatch::run(args) {
        Ok(()) => ExitCode::SUCCESS,
        // `dispatch::run` still flattens subcommand failures to a `String`
        // (legacy paths preserved). Surface them through the canonical
        // structured envelope on stderr with exit 2; `emit` never returns.
        Err(message) => CliError::cli(message).emit(),
    }
}
