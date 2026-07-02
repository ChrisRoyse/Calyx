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
        info: calyx_buildinfo::build_info!(),
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
        let info: BuildInfo = calyx_buildinfo::build_info!();
        let computed = calyx_buildinfo::compute_for_dir(env!("CARGO_MANIFEST_DIR"))
            .expect("compute identity in the real checkout");
        assert_eq!(info.git_sha, computed.git_sha);
        assert_eq!(info.package, "calyx-cli");
    }
}
