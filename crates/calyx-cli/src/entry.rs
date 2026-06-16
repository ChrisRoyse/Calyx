use std::{env, process::ExitCode};

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
    if let Some(result) = crate::cmd::try_run(&args) {
        return match result {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => error.emit(),
        };
    }
    match crate::dispatch::run(args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => error.emit(),
    }
}
