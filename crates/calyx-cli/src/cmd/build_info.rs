//! `calyx build-info` prints the embedded build identity as JSON (#1108).
//!
//! Deploy tooling compares `git_sha` against `origin/main` to detect a stale
//! runner binary, so this command must never touch a vault, the panel, or the
//! GPU — it reads only compile-time constants plus the executable path.

use calyx_buildinfo::BuildInfo;
use calyx_core::CalyxError;
use serde::Serialize;

use crate::error::{CliError, CliResult};
use crate::output::print_json;

#[derive(Debug, Serialize)]
struct BuildInfoReport {
    binary: &'static str,
    #[serde(flatten)]
    info: BuildInfo,
    executable: String,
}

pub(crate) fn try_run(args: &[String]) -> Option<CliResult> {
    let (command, rest) = args.split_first()?;
    if command != "build-info" {
        return None;
    }
    if let [flag] = rest
        && matches!(flag.as_str(), "--help" | "-h")
    {
        return Some(crate::usage::print_command_usage(command));
    }
    Some(run(rest))
}

fn run(rest: &[String]) -> CliResult {
    if !rest.is_empty() {
        return Err(CliError::usage(format!(
            "build-info takes no arguments, got {:?}",
            rest.join(" ")
        )));
    }
    let executable = std::env::current_exe().map_err(|error| {
        CliError::from(CalyxError::forge_device_unavailable(format!(
            "current executable path unavailable: {error}"
        )))
    })?;
    print_json(&BuildInfoReport {
        binary: "calyx",
        info: calyx_buildinfo::build_info!(capabilities: crate::capabilities::COMPILED),
        executable: executable.display().to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn try_run_ignores_other_commands() {
        assert!(try_run(&["search".to_string()]).is_none());
        assert!(try_run(&[]).is_none());
    }

    #[test]
    fn build_info_rejects_extra_arguments() {
        let result = try_run(&["build-info".to_string(), "--vault".to_string()])
            .expect("build-info owns its argument errors");
        let error = result.expect_err("extra arguments must be an error");
        assert!(
            format!("{error:?}").contains("takes no arguments"),
            "{error:?}"
        );
    }

    #[test]
    fn embedded_identity_matches_this_checkout() {
        let info: BuildInfo =
            calyx_buildinfo::build_info!(capabilities: crate::capabilities::COMPILED);
        let computed = calyx_buildinfo::compute_for_dir(env!("CARGO_MANIFEST_DIR"))
            .expect("compute identity in the real checkout");
        assert_eq!(info.git_sha, computed.git_sha);
        assert_eq!(info.package, "calyx-cli");
    }

    /// #1130 invariant: with the unified `cuda` feature, either the whole GPU
    /// surface is compiled in or none of it is — a partial surface (e.g.
    /// forge-cuda without sextant-cuvs on the Linux deploy target) is exactly
    /// the blind spot that shipped. Non-Linux targets legitimately compile
    /// forge/registry CUDA without cuVS (RAPIDS is Linux-only, #1016), so the
    /// all-or-nothing assertion is Linux-scoped.
    #[test]
    fn capabilities_are_all_or_nothing_for_the_unified_cuda_feature() {
        let info: BuildInfo =
            calyx_buildinfo::build_info!(capabilities: crate::capabilities::COMPILED);
        let cuda_requested = info.features.contains(&"cuda");
        for (name, compiled) in [
            ("forge-cuda", true),
            ("registry-candle-cuda", true),
            ("sextant-cuvs", cfg!(target_os = "linux")),
        ] {
            let expected = cuda_requested && compiled;
            assert_eq!(
                info.capabilities.get(name),
                Some(&expected),
                "capability {name}: features={:?} capabilities={:?}",
                info.features,
                info.capabilities
            );
        }
    }
}
