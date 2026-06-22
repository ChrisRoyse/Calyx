//! `calyx healthcheck --config <calyx.toml> [--wait <secs>] [--out <json>]`
//! (PH65 · T04 — daemon-readiness mode).
//!
//! This is the operational-readiness probe the PH66 systemd unit runs as
//! `ExecStartPost`: it does a real CUDA init, honors the configured VRAM budget
//! against live NVML usage, and reads the configured Aster vault back — then
//! writes the [`CalyxHealthResult`] JSON to the config's `health_log_path`.
//!
//! It owns its exit codes ahead of the generic CLI matcher (like
//! [`crate::verify_restore`]), per the daemon contract:
//! `0` = every probe healthy · `1` = ran but unhealthy (the JSON records the
//! exact `CALYX_*` code) · `2` = could not run (bad args/config, or the SoT
//! could not be written). The plain `calyx healthcheck` deploy-health command
//! (no `--config`) is untouched — this only intercepts the `--config` form.

use std::path::PathBuf;
use std::process::ExitCode;

use calyxd::config::CalyxConfig;
use calyxd::health::{CalyxHealthResult, run_healthcheck, run_with_wait, write_health_result};

const USAGE: &str = "usage: calyx healthcheck --config <calyx.toml> [--wait <secs>] [--out <json>]";

/// Intercepts `healthcheck --config …`; returns `None` for the plain
/// deploy-health `healthcheck` form and every other command.
pub fn try_run(args: &[String]) -> Option<ExitCode> {
    let (first, rest) = args.split_first()?;
    if first != "healthcheck" || !rest.iter().any(|arg| arg == "--config") {
        return None;
    }
    Some(run(rest))
}

/// Parsed daemon-readiness invocation.
#[derive(Debug, PartialEq, Eq)]
struct Args {
    config: PathBuf,
    /// `--wait` override; `None` falls back to `healthcheck_timeout_secs`.
    wait_secs: Option<u32>,
    /// `--out` override; `None` falls back to the config's `health_log_path`.
    out: Option<PathBuf>,
}

/// The result of a readiness probe, separated from process exit so it is unit
/// testable (a `process::exit` would kill the test runner).
enum Outcome {
    Healthy(CalyxHealthResult),
    Unhealthy(CalyxHealthResult),
    CannotRun(String),
}

fn run(rest: &[String]) -> ExitCode {
    match evaluate(rest) {
        Outcome::Healthy(result) => {
            println!(
                "HEALTHCHECK pass cuda_device={} vram_budget_mib={} vault_read_ok={}",
                result.cuda_device.as_deref().unwrap_or("<none>"),
                result.vram_budget_mib,
                result.vault_read_ok
            );
            ExitCode::SUCCESS
        }
        Outcome::Unhealthy(result) => {
            eprintln!(
                "HEALTHCHECK fail error_code={} detail={}",
                result
                    .error_code
                    .as_deref()
                    .unwrap_or("CALYX_DAEMON_HEALTH_FAIL"),
                result.error_detail.as_deref().unwrap_or("<none>")
            );
            ExitCode::from(1)
        }
        Outcome::CannotRun(message) => {
            eprintln!("error: {message}");
            ExitCode::from(2)
        }
    }
}

fn evaluate(rest: &[String]) -> Outcome {
    let args = match parse(rest) {
        Ok(args) => args,
        Err(message) => return Outcome::CannotRun(message),
    };
    let cfg = match CalyxConfig::from_file(&args.config) {
        Ok(cfg) => cfg,
        Err(error) => return Outcome::CannotRun(error.to_string()),
    };
    let wait_secs = args.wait_secs.unwrap_or(cfg.healthcheck_timeout_secs);
    let (result, _attempts) = run_with_wait(wait_secs, || run_healthcheck(&cfg));
    let out = args.out.unwrap_or_else(|| cfg.health_log_path.clone());
    if let Err(error) = write_health_result(&result, &out) {
        return Outcome::CannotRun(error.to_string());
    }
    if result.is_pass() {
        Outcome::Healthy(result)
    } else {
        Outcome::Unhealthy(result)
    }
}

fn parse(rest: &[String]) -> Result<Args, String> {
    let mut config = None;
    let mut wait_secs = None;
    let mut out = None;
    let mut iter = rest.iter();
    while let Some(flag) = iter.next() {
        match flag.as_str() {
            "--config" => config = Some(PathBuf::from(value(flag, iter.next())?)),
            "--out" => out = Some(PathBuf::from(value(flag, iter.next())?)),
            "--wait" => {
                let raw = value(flag, iter.next())?;
                wait_secs = Some(
                    raw.parse::<u32>()
                        .map_err(|error| format!("{USAGE}\n--wait {raw}: {error}"))?,
                );
            }
            other => return Err(format!("{USAGE}\nunknown argument {other}")),
        }
    }
    let config = config.ok_or_else(|| format!("{USAGE}\n--config is required"))?;
    Ok(Args {
        config,
        wait_secs,
        out,
    })
}

fn value<'a>(flag: &str, next: Option<&'a String>) -> Result<&'a str, String> {
    next.map(String::as_str)
        .ok_or_else(|| format!("{USAGE}\n{flag} requires a value"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    #[test]
    fn try_run_only_intercepts_the_config_form() {
        // Plain deploy-health healthcheck (no --config) falls through.
        assert!(try_run(&args(&["healthcheck", "--out", "/tmp/x.json"])).is_none());
        // Other commands are ignored.
        assert!(try_run(&args(&["verify-restore", "--vault", "/tmp/v"])).is_none());
        assert!(try_run(&args(&[])).is_none());
    }

    #[test]
    fn parse_requires_config_and_reads_optional_flags() {
        let parsed = parse(&args(&[
            "--config",
            "/etc/calyx.toml",
            "--wait",
            "5",
            "--out",
            "/tmp/h.json",
        ]))
        .expect("valid args");
        assert_eq!(
            parsed,
            Args {
                config: PathBuf::from("/etc/calyx.toml"),
                wait_secs: Some(5),
                out: Some(PathBuf::from("/tmp/h.json")),
            }
        );
        assert!(
            parse(&args(&["--wait", "5"]))
                .unwrap_err()
                .contains("--config is required")
        );
        assert!(
            parse(&args(&["--config"]))
                .unwrap_err()
                .contains("requires a value")
        );
        assert!(
            parse(&args(&["--config", "/c", "--bogus"]))
                .unwrap_err()
                .contains("unknown argument")
        );
    }

    #[test]
    fn missing_config_file_cannot_run_exit_2() {
        let outcome = evaluate(&args(&[
            "--config",
            "/no/such/calyx.toml-xyz",
            "--wait",
            "0",
        ]));
        assert!(matches!(outcome, Outcome::CannotRun(_)));
    }

    // On the gate (calyxd built WITHOUT the `cuda` feature) the CUDA probe fails
    // loud, so a real readiness run is Unhealthy and STILL writes the fail JSON
    // to the SoT — exercising parse → run_healthcheck → write_health_result end
    // to end without a GPU. The hardware pass-path is the manual FSV.
    #[test]
    fn unhealthy_run_writes_fail_json_to_sot() {
        let dir = std::env::temp_dir().join(format!("calyx-hcd-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("mkdir");
        let out = dir.join("latest.json");
        let toml = dir.join("calyx.toml");
        std::fs::write(
            &toml,
            "vault_path = \"/no/such/vault-xyz\"\nvram_budget_mib = 8192\nlog_dir = \"/tmp\"\n",
        )
        .expect("write toml");

        let outcome = evaluate(&args(&[
            "--config",
            &toml.display().to_string(),
            "--wait",
            "0",
            "--out",
            &out.display().to_string(),
        ]));

        match outcome {
            Outcome::Unhealthy(result) => {
                assert_eq!(result.status, "fail");
                assert!(
                    result.error_code.is_some(),
                    "fail must carry a CALYX_* code"
                );
            }
            Outcome::Healthy(_) => panic!("no GPU + missing vault must not be healthy"),
            Outcome::CannotRun(message) => panic!("should have run and written JSON: {message}"),
        }
        // The SoT file physically exists and is the fail record.
        let text = std::fs::read_to_string(&out).expect("read SoT");
        let value: serde_json::Value = serde_json::from_str(&text).expect("valid JSON");
        assert_eq!(value["status"], "fail");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
