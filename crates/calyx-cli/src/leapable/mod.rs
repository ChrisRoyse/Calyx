mod issue612_fsv;
pub(crate) mod shadow_harness;
mod shadow_harness_cli;
#[cfg(test)]
mod shadow_harness_tests;

pub(crate) use shadow_harness::{ShadowVault, VaultMode};

pub(crate) fn readback_shadow_manifest(vault: &std::path::Path) -> crate::error::CliResult {
    shadow_harness_cli::readback_shadow_manifest_cli(vault)
}

pub(crate) fn run(topic: &str, args: &[String]) -> crate::error::CliResult {
    match topic {
        "issue612-fsv" => issue612_fsv::run(args),
        "shadow-open" => shadow_harness_cli::run_shadow_open(args),
        "shadow-readback" => shadow_harness_cli::run_shadow_readback(args),
        _ => Err(format!("unknown leapable command: {topic}").into()),
    }
}
