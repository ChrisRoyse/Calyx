//! Calyx daemon: Ledger chain-verify metrics on a loopback `/metrics` endpoint.
//!
//! The first verify cycle runs synchronously before the listener binds, so a
//! scrape can never observe an unverified gauge. Misconfiguration exits with
//! `CALYX_DAEMON_CONFIG_INVALID`; a non-loopback bind exits with
//! `CALYX_DAEMON_BIND_FAILED`. A broken/corrupt/unverifiable chain is not an
//! exit — it is the alert: the gauge holds 0 until the chain verifies intact.

// Shared daemon modules (config, error, the T02 CUDA probe, the T03 VRAM
// budget, the PH66 T03 metrics surface) live in the `calyxd` library — the
// single source of truth, reused by `calyx-cli` and the T04 healthcheck. The
// binary consumes them from the lib rather than recompiling its own copies.
// `verify_loop` is the binary-only periodic chain-verify driver.
mod startup;
mod verify_loop;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use calyxd::error::DaemonError;
use calyxd::metrics::{CalyxMetrics, ChainVerifyMetrics, collect_default_zfs_integrity};
use calyxd::server::MetricsServer;
use startup::{run_server, validate_config};
use tokio_util::sync::CancellationToken;
use verify_loop::{TargetKind, VerifyTarget, run_cycle, spawn_loop};

const USAGE: &str = "usage: calyxd (--vault <dir> | --ledger <dir>)... \
[--bind <loopback-addr:port>] [--interval-secs <n>] [--once]
       calyxd --config <calyx.toml> --validate-config
  --vault <dir>        Aster vault directory to chain-verify (repeatable)
  --ledger <dir>       standalone directory ledger to chain-verify (repeatable)
  --bind <addr>        loopback listen address (default 127.0.0.1:7700)
  --interval-secs <n>  seconds between verify cycles (default 60, min 1)
  --once               run one verify cycle, print metrics text, exit
  --config <path>      path to a calyx.toml runtime config file
  --validate-config    parse+validate --config, print it (no secrets), exit
  --audit-vram         with --config: CUDA preflight + NVML VRAM audit, then exit";

#[derive(Debug)]
struct Config {
    targets: Vec<VerifyTarget>,
    bind: SocketAddr,
    interval: Duration,
    once: bool,
    config_path: Option<PathBuf>,
    validate_config: bool,
    audit_vram: bool,
}

#[tokio::main]
async fn main() -> ExitCode {
    let config = match parse_args(std::env::args().skip(1).collect()) {
        Ok(config) => config,
        Err(error) => {
            eprintln!("calyxd: {error}\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    if config.validate_config {
        return validate_config(config.config_path.as_deref());
    }
    // Server mode: a --config (without --validate-config) boots the config-driven
    // daemon, which begins with a fatal CUDA preflight before any other init.
    if let Some(path) = config.config_path.clone() {
        return run_server(&path, config.once, config.audit_vram).await;
    }
    match run(config) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("calyxd: {error}");
            ExitCode::from(2)
        }
    }
}

fn run(config: Config) -> Result<(), DaemonError> {
    for target in &config.targets {
        target.validate()?;
    }
    let labels = config
        .targets
        .iter()
        .map(VerifyTarget::label)
        .collect::<Vec<_>>();
    let chain = Arc::new(ChainVerifyMetrics::new(&labels));

    run_cycle(&config.targets, &chain);

    // The served surface composes the live chain-verify family (updated in place
    // by the verify loop) with the full PH66 T03 metric set. Both share the same
    // `chain` Arc, so a scrape reflects the latest verify cycle.
    let surface = Arc::new(CalyxMetrics::new(Arc::clone(&chain), &labels));
    refresh_zfs_metrics(&surface);

    if config.once {
        let text = surface.encode_text().map_err(DaemonError::config_invalid)?;
        print!("{text}");
        return Ok(());
    }

    let server = MetricsServer::bind(config.bind, Arc::clone(&surface))?;
    println!(
        "calyxd: serving /metrics on {} (verify interval {}s, {} target(s))",
        server.local_addr()?,
        config.interval.as_secs(),
        config.targets.len()
    );
    spawn_loop(config.targets, chain, config.interval);
    spawn_zfs_metrics_loop(Arc::clone(&surface), config.interval);
    server.run(CancellationToken::new())
}

fn refresh_zfs_metrics(metrics: &CalyxMetrics) {
    match collect_default_zfs_integrity() {
        Ok(snapshot) => metrics.record_zfs_integrity(&snapshot),
        Err(detail) => eprintln!("calyxd: zfs integrity metrics refresh failed: {detail}"),
    }
}

fn spawn_zfs_metrics_loop(metrics: Arc<CalyxMetrics>, interval: Duration) {
    let _zfs_thread = std::thread::Builder::new()
        .name("calyxd-zfs-metrics".to_string())
        .spawn(move || {
            loop {
                std::thread::sleep(interval);
                refresh_zfs_metrics(&metrics);
            }
        })
        .expect("spawn zfs metrics loop");
}

fn parse_args(args: Vec<String>) -> Result<Config, DaemonError> {
    let mut targets = Vec::new();
    let mut bind: SocketAddr = "127.0.0.1:7700"
        .parse()
        .expect("default bind address parses");
    let mut interval = Duration::from_secs(60);
    let mut once = false;
    let mut config_path = None;
    let mut validate_config = false;
    let mut audit_vram = false;

    let mut iter = args.into_iter();
    while let Some(flag) = iter.next() {
        match flag.as_str() {
            "--config" => {
                let value = require_value(&flag, iter.next())?;
                config_path = Some(PathBuf::from(value));
            }
            "--validate-config" => validate_config = true,
            "--audit-vram" => audit_vram = true,
            "--vault" | "--ledger" => {
                let path = require_value(&flag, iter.next())?;
                let kind = if flag == "--vault" {
                    TargetKind::Vault
                } else {
                    TargetKind::LedgerDir
                };
                targets.push(VerifyTarget {
                    kind,
                    path: PathBuf::from(path),
                });
            }
            "--bind" => {
                let value = require_value(&flag, iter.next())?;
                bind = value.parse().map_err(|error| {
                    DaemonError::config_invalid(format!("--bind {value}: {error}"))
                })?;
            }
            "--interval-secs" => {
                let value = require_value(&flag, iter.next())?;
                let secs: u64 = value.parse().map_err(|error| {
                    DaemonError::config_invalid(format!("--interval-secs {value}: {error}"))
                })?;
                if secs == 0 {
                    return Err(DaemonError::config_invalid("--interval-secs must be >= 1"));
                }
                interval = Duration::from_secs(secs);
            }
            "--once" => once = true,
            other => {
                return Err(DaemonError::config_invalid(format!(
                    "unknown argument {other}"
                )));
            }
        }
    }

    if (validate_config || audit_vram) && config_path.is_none() {
        config_path = Some(PathBuf::from("calyx.toml"));
    }
    // `--validate-config`, `--audit-vram`, and server mode (`--config <path>`)
    // need no explicit verify targets — the config supplies them.
    if !validate_config && config_path.is_none() && targets.is_empty() {
        return Err(DaemonError::config_invalid(
            "at least one --vault or --ledger target is required",
        ));
    }
    Ok(Config {
        targets,
        bind,
        interval,
        once,
        config_path,
        validate_config,
        audit_vram,
    })
}

fn require_value(flag: &str, value: Option<String>) -> Result<String, DaemonError> {
    value.ok_or_else(|| DaemonError::config_invalid(format!("{flag} requires a value")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    #[test]
    fn parse_args_requires_at_least_one_target() {
        let error = parse_args(args(&[])).unwrap_err();
        assert_eq!(error.code(), "CALYX_DAEMON_CONFIG_INVALID");
        assert!(error.to_string().contains("--vault or --ledger"));
    }

    #[test]
    fn parse_args_defaults_bind_to_loopback_7700() {
        let config = parse_args(args(&["--vault", "/data/v"])).unwrap();
        assert_eq!(config.bind, "127.0.0.1:7700".parse().unwrap());
        assert_eq!(config.interval, Duration::from_secs(60));
        assert!(!config.once);
        assert_eq!(config.targets.len(), 1);
        assert_eq!(config.targets[0].kind, TargetKind::Vault);
    }

    #[test]
    fn parse_args_rejects_zero_interval_and_unknown_flags() {
        assert!(
            parse_args(args(&["--vault", "/v", "--interval-secs", "0"]))
                .unwrap_err()
                .to_string()
                .contains(">= 1")
        );
        assert!(
            parse_args(args(&["--vault", "/v", "--bogus"]))
                .unwrap_err()
                .to_string()
                .contains("unknown argument --bogus")
        );
    }

    #[test]
    fn parse_args_rejects_invalid_bind_value() {
        let error = parse_args(args(&["--vault", "/v", "--bind", "not-an-addr"])).unwrap_err();
        assert_eq!(error.code(), "CALYX_DAEMON_CONFIG_INVALID");
        assert!(error.to_string().contains("not-an-addr"));
    }

    #[test]
    fn run_rejects_missing_target_directory_fail_closed() {
        let config = parse_args(args(&["--vault", "Z:/missing/vault-602", "--once"])).unwrap();
        let error = run(config).unwrap_err();
        assert_eq!(error.code(), "CALYX_DAEMON_CONFIG_INVALID");
    }

    #[test]
    fn parse_args_validate_config_needs_no_target() {
        let config = parse_args(args(&["--config", "calyx.toml", "--validate-config"]))
            .expect("validate-config mode requires no verify target");
        assert!(config.validate_config);
        assert_eq!(config.config_path, Some(PathBuf::from("calyx.toml")));
        assert!(config.targets.is_empty());
    }

    #[test]
    fn parse_args_config_requires_value() {
        let error = parse_args(args(&["--config"])).unwrap_err();
        assert_eq!(error.code(), "CALYX_DAEMON_CONFIG_INVALID");
        assert!(error.to_string().contains("--config requires a value"));
    }
}
