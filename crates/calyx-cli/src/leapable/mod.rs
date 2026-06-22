pub(crate) mod dual_write;
#[cfg(test)]
mod dual_write_tests;
mod issue612_fsv;
pub(crate) mod panel_guard_enable;
pub(crate) mod production_fsv;
#[cfg(test)]
mod production_fsv_tests;
pub(crate) mod read_flip;
#[cfg(test)]
mod read_flip_tests;
pub(crate) mod recall_comparator;
pub(crate) mod round_trip_verifier;
#[cfg(test)]
mod round_trip_verifier_tests;
pub(crate) mod shadow_harness;
mod shadow_harness_cli;
#[cfg(test)]
mod shadow_harness_tests;
pub(crate) mod shadow_removal;
#[cfg(test)]
mod shadow_removal_tests;

pub(crate) use shadow_harness::{ShadowVault, VaultMode};

pub(crate) fn readback_shadow_manifest(vault: &std::path::Path) -> crate::error::CliResult {
    shadow_harness_cli::readback_shadow_manifest_cli(vault)
}

pub(crate) fn readback_dual_write_verify(
    vault: &std::path::Path,
    sqlite: &std::path::Path,
) -> crate::error::CliResult {
    dual_write::run_readback_verify(vault, sqlite)
}

pub(crate) fn run(topic: &str, args: &[String]) -> crate::error::CliResult {
    match topic {
        "ask" => read_flip::run_ask(args),
        "dual-write" => dual_write::run_dual_write(args),
        "issue612-fsv" => issue612_fsv::run(args),
        "production-fsv" => production_fsv::run_production_fsv(args),
        "read-flip" => read_flip::run_read_flip(args),
        "recall-compare" => recall_comparator::run_recall_compare(args),
        "remove-shadow" => shadow_removal::run_remove_shadow(args),
        "verify-round-trip" => round_trip_verifier::run_verify_round_trip(args),
        "shadow-open" => shadow_harness_cli::run_shadow_open(args),
        "shadow-readback" => shadow_harness_cli::run_shadow_readback(args),
        _ => Err(format!("unknown leapable command: {topic}").into()),
    }
}
