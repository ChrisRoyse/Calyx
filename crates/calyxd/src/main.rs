//! Calyx daemon: Ledger chain-verify metrics on a loopback `/metrics` endpoint.
//!
//! The first verify cycle runs synchronously before the listener binds, so a
//! scrape can never observe an unverified gauge. Misconfiguration exits with
//! `CALYX_DAEMON_CONFIG_INVALID`; a non-loopback bind exits with
//! `CALYX_DAEMON_BIND_FAILED`. A broken/corrupt/unverifiable chain is not an
//! exit — it is the alert: the gauge holds 0 until the chain verifies intact.

mod config;
mod cuda_probe;
mod error;
mod metrics;
mod server;
mod verify_loop;
mod vram;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use config::CalyxConfig;
use error::DaemonError;
use metrics::ChainVerifyMetrics;
use server::MetricsServer;
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

fn main() -> ExitCode {
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
        return run_server(&path, config.once, config.audit_vram);
    }
    match run(config) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("calyxd: {error}");
            ExitCode::from(2)
        }
    }
}

/// Server mode: load `calyx.toml`, run the fatal CUDA preflight (T02) and the
/// VRAM budget audit honoring resident TEI (T03), then chain-verify the
/// configured vault on the configured loopback. `--audit-vram` stops after the
/// audit (no vault needed). T05/T06 add the MCP dispatch surface.
///
/// A CUDA or NVML failure — or an already-exhausted budget — is fatal with exit
/// code 1 and a structured `CALYX_*` code; there is no CPU fallback.
fn run_server(config_path: &std::path::Path, once: bool, audit_vram: bool) -> ExitCode {
    let cfg = match CalyxConfig::from_file(config_path) {
        Ok(cfg) => cfg,
        Err(error) => {
            eprintln!("calyxd: {error}");
            return ExitCode::from(2);
        }
    };
    let device = match cuda_probe::probe_cuda_device() {
        Ok(device) => device,
        Err(error) => {
            eprintln!("calyxd: {error}");
            return ExitCode::from(1);
        }
    };
    println!(
        "INFO calyxd: CUDA device ready device=\"{}\" vram={}MiB compute={}",
        device.device_name, device.vram_total_mib, device.compute_cap
    );
    // PH65 T03: VRAM budget audit against live NVML usage, honoring resident TEI.
    let nvml = match vram::NvmlVramUsage::init() {
        Ok(nvml) => nvml,
        Err(error) => {
            eprintln!("calyxd: {error}");
            return ExitCode::from(1);
        }
    };
    let budget = match vram::VramBudget::from_config(cfg.vram_budget_mib, &device, nvml) {
        Ok(budget) => budget,
        Err(error) => {
            eprintln!("calyxd: {error}");
            return ExitCode::from(1);
        }
    };
    let audit = match budget.startup_vram_audit() {
        Ok(audit) => audit,
        Err(error) => {
            eprintln!("calyxd: {error}");
            return ExitCode::from(1);
        }
    };
    println!(
        "INFO calyxd: VRAM audit tei_used={}MiB calyx_budget={}MiB device_total={}MiB available={}MiB",
        audit.tei_used_mib,
        audit.calyx_budget_mib,
        audit.device_total_mib,
        audit.calyx_budget_mib.saturating_sub(audit.tei_used_mib)
    );
    // Startup readiness signal: would a representative Forge dispatch be admitted
    // right now? Per-dispatch enforcement (the hard gate) happens at dispatch
    // time once Forge work is wired (PH65 T05/T06); this is observability, not a
    // hard gate, because resident usage fluctuates.
    const PROBE_DISPATCH_MIB: u32 = 256;
    let available = match budget.available_mib() {
        Ok(available) => available,
        Err(error) => {
            eprintln!("calyxd: {error}");
            return ExitCode::from(1);
        }
    };
    let dispatch_ready = budget.check_can_allocate(PROBE_DISPATCH_MIB).is_ok();
    println!(
        "INFO calyxd: dispatch readiness available={available}MiB probe={PROBE_DISPATCH_MIB}MiB admitted={dispatch_ready}"
    );
    if audit_vram {
        return ExitCode::SUCCESS;
    }
    let server_config = Config {
        targets: vec![VerifyTarget {
            kind: TargetKind::Vault,
            path: cfg.vault_path_resolved(),
        }],
        bind: cfg.bind_addr,
        interval: Duration::from_secs(60),
        once,
        config_path: None,
        validate_config: false,
        audit_vram: false,
    };
    match run(server_config) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("calyxd: {error}");
            ExitCode::from(2)
        }
    }
}

/// `--validate-config`: load the reference TOML, run fail-closed validation, and
/// print the parsed config (which holds no secrets). Exits 0 on success, 2 with
/// the stable `CALYX_*` error code on any failure.
fn validate_config(path: Option<&std::path::Path>) -> ExitCode {
    let Some(path) = path else {
        eprintln!(
            "calyxd: {}",
            DaemonError::config_invalid("--validate-config requires --config <path>")
        );
        return ExitCode::from(2);
    };
    match CalyxConfig::from_file(path) {
        Ok(config) => {
            println!("calyxd: config {} OK", path.display());
            println!("{config:#?}");
            println!(
                "calyxd: vault_path_resolved = {}",
                config.vault_path_resolved().display()
            );
            ExitCode::SUCCESS
        }
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
    let metrics = Arc::new(ChainVerifyMetrics::new(&labels));

    run_cycle(&config.targets, &metrics);

    if config.once {
        let text = metrics.encode_text().map_err(DaemonError::config_invalid)?;
        print!("{text}");
        return Ok(());
    }

    let server = MetricsServer::bind(config.bind, Arc::clone(&metrics))?;
    println!(
        "calyxd: serving /metrics on {} (verify interval {}s, {} target(s))",
        server.local_addr()?,
        config.interval.as_secs(),
        config.targets.len()
    );
    spawn_loop(config.targets, metrics, config.interval);
    server.run()
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

    // `--validate-config` and server mode (`--config <path>`) need no explicit
    // verify targets — the config supplies them.
    if !validate_config && config_path.is_none() && targets.is_empty() {
        return Err(DaemonError::config_invalid(
            "at least one --vault or --ledger target is required",
        ));
    }
    if audit_vram && config_path.is_none() {
        return Err(DaemonError::config_invalid(
            "--audit-vram requires --config <calyx.toml>",
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
        let config = parse_args(args(&[
            "--config",
            "infra/aiwonder/calyx.toml",
            "--validate-config",
        ]))
        .expect("validate-config mode requires no verify target");
        assert!(config.validate_config);
        assert_eq!(
            config.config_path,
            Some(PathBuf::from("infra/aiwonder/calyx.toml"))
        );
        assert!(config.targets.is_empty());
    }

    #[test]
    fn parse_args_config_requires_value() {
        let error = parse_args(args(&["--config"])).unwrap_err();
        assert_eq!(error.code(), "CALYX_DAEMON_CONFIG_INVALID");
        assert!(error.to_string().contains("--config requires a value"));
    }
}
