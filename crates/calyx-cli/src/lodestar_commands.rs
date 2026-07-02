use crate::error::CliError;

pub(crate) fn run(topic: &str, args: &[String]) -> crate::error::CliResult {
    match topic {
        "kernel-validate" => super::lodestar_kernel_validation::run(args),
        other => Err(CliError::usage(format!("unknown lodestar topic: {other}"))),
    }
}
