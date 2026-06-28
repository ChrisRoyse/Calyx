use std::path::Path;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use calyxd::config::CalyxConfig;
use calyxd::cuda_probe;
use calyxd::error::DaemonError;
use calyxd::health::{run_healthcheck, write_health_result, write_shutdown_status};
use calyxd::learner_origin::LearnerOriginService;
use calyxd::metrics::{CalyxMetrics, ChainVerifyMetrics};
use calyxd::server::MetricsServer;
use calyxd::verify::{VerifyRestoreReport, verify_restore};
use calyxd::vram::{self, NvmlVramUsage};
use tokio_util::sync::CancellationToken;

use crate::verify_loop::{TargetKind, VerifyTarget, run_cycle, spawn_loop};
use crate::{refresh_zfs_metrics, spawn_zfs_metrics_loop};

const VERIFY_INTERVAL_SECS: u64 = 60;

pub(crate) async fn run_server(config_path: &Path, once: bool, audit_vram: bool) -> ExitCode {
    let cfg = match CalyxConfig::from_file(config_path) {
        Ok(cfg) => cfg,
        Err(error) => return fatal(error),
    };
    let device = match cuda_probe::probe_cuda_device() {
        Ok(device) => device,
        Err(error) => return fatal(error),
    };
    let budget = match build_vram_budget(&cfg, &device) {
        Ok(budget) => budget,
        Err(error) => return fatal(error),
    };
    let audit = match budget.startup_vram_audit() {
        Ok(audit) => audit,
        Err(error) => return fatal(error),
    };

    if audit_vram {
        match serde_json::to_string_pretty(&audit) {
            Ok(json) => println!("{json}"),
            Err(error) => {
                return fatal(DaemonError::health_failed(format!(
                    "serialize VRAM audit: {error}"
                )));
            }
        }
        return ExitCode::SUCCESS;
    }

    let vault_path = cfg.vault_path_resolved();
    let restore_report = match verify_vault_for_startup(&vault_path) {
        Ok(report) => report,
        Err(error) => return fatal(error),
    };

    let target = VerifyTarget {
        kind: TargetKind::Vault,
        path: vault_path.clone(),
    };
    let labels = vec![target.label()];
    let chain = Arc::new(ChainVerifyMetrics::new(&labels));
    run_cycle(std::slice::from_ref(&target), &chain);
    let surface = Arc::new(CalyxMetrics::new(Arc::clone(&chain), &labels));
    let vault_label = target.label();
    surface.record_vram_budget_audit(&vault_label, "runtime", &audit);
    surface.record_verify_restore(&vault_label, &restore_report, unix_now_secs());
    refresh_zfs_metrics(&surface);
    // #1934: surface the configured VRAM budget ceiling on /metrics. The limit is
    // the static configured ceiling from calyx.toml (always known, independent of
    // GPU mode), sourced from the real startup VRAM audit. calyxd runs CPU-only and
    // reserves no VRAM of its own budget, so used is 0 — an honest reading; the
    // device-wide TEI footprint is a separate concern, not Calyx budget consumption.
    surface.set_vram_budget(0, i64::from(audit.calyx_budget_mib));
    let origin = match cfg.learner_origin.as_ref() {
        Some(origin_cfg) => match LearnerOriginService::from_config(origin_cfg) {
            Ok(service) => Some(Arc::new(service)),
            Err(error) => return fatal(error),
        },
        None => None,
    };

    if once {
        return print_once(&surface, origin.as_deref());
    }

    let server = match &origin {
        Some(origin) => {
            MetricsServer::bind_with_origin(cfg.bind_addr, Arc::clone(&surface), Arc::clone(origin))
        }
        None => MetricsServer::bind(cfg.bind_addr, Arc::clone(&surface)),
    };
    let server = match server {
        Ok(server) => server,
        Err(error) => return fatal(error),
    };
    let cancel_token = CancellationToken::new();
    if let Err(error) = install_signal_handlers(cancel_token.clone()) {
        return fatal(error);
    }

    let health = run_healthcheck(&cfg);
    if let Err(error) = write_health_result(&health, &cfg.health_log_path) {
        return fatal(error);
    }
    if !health.is_pass() {
        eprintln!(
            "calyxd: CALYX_DAEMON_HEALTH_FAIL: startup healthcheck failed; listener will not accept"
        );
        return ExitCode::from(1);
    }

    println!(
        "INFO calyxd {} starting device=\"{}\" vram_budget={}MiB bind={} vault={} learner_origin={}",
        env!("CARGO_PKG_VERSION"),
        device.device_name,
        cfg.vram_budget_mib,
        cfg.bind_addr,
        vault_path.display(),
        origin.is_some()
    );
    spawn_loop(
        vec![target],
        chain,
        Duration::from_secs(VERIFY_INTERVAL_SECS),
    );
    spawn_zfs_metrics_loop(
        Arc::clone(&surface),
        Duration::from_secs(VERIFY_INTERVAL_SECS),
    );

    match server.run(cancel_token) {
        Ok(()) => match write_shutdown_status(&cfg.health_log_path) {
            Ok(record) => {
                println!(
                    "INFO calyxd shutdown status={} timestamp_utc={}",
                    record.status, record.timestamp_utc
                );
                ExitCode::SUCCESS
            }
            Err(error) => fatal(error),
        },
        Err(error) => fatal(error),
    }
}

pub(crate) fn validate_config(path: Option<&Path>) -> ExitCode {
    let Some(path) = path else {
        return fatal(DaemonError::config_invalid(
            "--validate-config requires --config <path>",
        ));
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
        Err(error) => fatal(error),
    }
}

fn build_vram_budget(
    cfg: &CalyxConfig,
    device: &cuda_probe::CudaDeviceInfo,
) -> Result<vram::VramBudget<NvmlVramUsage>, DaemonError> {
    let nvml = NvmlVramUsage::init()?;
    vram::VramBudget::from_config(cfg.vram_budget_mib, device, nvml)
}

fn verify_vault_for_startup(path: &Path) -> Result<VerifyRestoreReport, DaemonError> {
    verify_restore(path).and_then(|report| {
        if report.success() {
            Ok(report)
        } else {
            Err(DaemonError::health_failed(format!(
                "vault {} startup read-back unverified: {}",
                path.display(),
                report.failure_reasons().join("; ")
            )))
        }
    })
}

fn unix_now_secs() -> i64 {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(elapsed) => i64::try_from(elapsed.as_secs()).unwrap_or(i64::MAX),
        Err(error) => {
            eprintln!("calyxd: system clock before unix epoch: {error}");
            0
        }
    }
}

fn print_once(surface: &CalyxMetrics, origin: Option<&LearnerOriginService>) -> ExitCode {
    match surface.encode_text() {
        Ok(mut text) => {
            if let Some(origin) = origin {
                match origin.metrics().encode_text() {
                    Ok(origin_text) => text.push_str(&origin_text),
                    Err(error) => return fatal(DaemonError::config_invalid(error)),
                }
            }
            print!("{text}");
            ExitCode::SUCCESS
        }
        Err(error) => fatal(DaemonError::config_invalid(error)),
    }
}

fn install_signal_handlers(cancel_token: CancellationToken) -> Result<(), DaemonError> {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        let mut sigint = signal(SignalKind::interrupt()).map_err(|error| {
            DaemonError::config_invalid(format!("install SIGINT handler: {error}"))
        })?;
        let mut sigterm = signal(SignalKind::terminate()).map_err(|error| {
            DaemonError::config_invalid(format!("install SIGTERM handler: {error}"))
        })?;
        tokio::spawn(async move {
            tokio::select! {
                _ = sigint.recv() => {}
                _ = sigterm.recv() => {}
            }
            cancel_token.cancel();
        });
    }

    #[cfg(not(unix))]
    {
        tokio::spawn(async move {
            if let Err(error) = tokio::signal::ctrl_c().await {
                eprintln!("calyxd: install Ctrl-C handler failed: {error}");
            }
            cancel_token.cancel();
        });
    }

    Ok(())
}

fn fatal(error: DaemonError) -> ExitCode {
    eprintln!("calyxd: {error}");
    ExitCode::from(1)
}
