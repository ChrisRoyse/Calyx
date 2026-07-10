use std::path::Path;
use std::process::ExitCode;
use std::sync::{Arc, mpsc};
use std::thread::{self, JoinHandle};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use calyx_mcp::McpServer;
use calyxd::config::CalyxConfig;
use calyxd::cuda_probe;
use calyxd::error::DaemonError;
use calyxd::health::{run_healthcheck, write_health_result, write_shutdown_status};
use calyxd::learner_origin::LearnerOriginService;
use calyxd::mcp_server::CalyxMcpServer;
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
        Some(origin) => MetricsServer::bind_with_origin_and_connection_limit(
            cfg.bind_addr,
            Arc::clone(&surface),
            Arc::clone(origin),
            cfg.max_metrics_connections,
        ),
        None => MetricsServer::bind_with_connection_limit(
            cfg.bind_addr,
            Arc::clone(&surface),
            cfg.max_metrics_connections,
        ),
    };
    let server = match server {
        Ok(server) => server,
        Err(error) => return fatal(error),
    };
    let mcp_server = match build_mcp_server(&cfg) {
        Ok(server) => server,
        Err(error) => return fatal(error),
    };
    let metrics_addr = match server.local_addr() {
        Ok(addr) => addr,
        Err(error) => return fatal(error),
    };
    let mcp_addr = match mcp_server.as_ref() {
        Some(server) => match server.local_addr() {
            Ok(addr) => Some(addr.to_string()),
            Err(error) => return fatal(error),
        },
        None => None,
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
        "INFO calyxd {} starting device=\"{}\" vram_budget={}MiB metrics_bind={} mcp_bind={} vault={} learner_origin={}",
        env!("CARGO_PKG_VERSION"),
        device.device_name,
        cfg.vram_budget_mib,
        metrics_addr,
        mcp_addr.as_deref().unwrap_or("disabled"),
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

    match run_servers(server, mcp_server, cancel_token).await {
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

fn build_mcp_server(cfg: &CalyxConfig) -> Result<Option<CalyxMcpServer>, DaemonError> {
    match (cfg.mcp_bind_addr, cfg.mcp_mtls.as_ref()) {
        (None, None) => return Ok(None),
        (None, Some(_)) => {
            return Err(DaemonError::config_invalid(
                "mcp_mtls is configured but mcp_bind_addr is missing; calyxd will not start MCP without an explicit loopback bind",
            ));
        }
        (Some(_), None) => {
            return Err(DaemonError::tls_config_invalid(
                "mcp_bind_addr is configured but mcp_mtls is missing; calyxd MCP requires mTLS",
            ));
        }
        (Some(_), Some(_)) => {}
    }
    let dispatcher = production_mcp_dispatcher()?;
    CalyxMcpServer::from_config(cfg, dispatcher).map(Some)
}

fn production_mcp_dispatcher() -> Result<Arc<McpServer>, DaemonError> {
    let mut dispatcher = McpServer::new();
    calyx_mcp::tools::register_all(&mut dispatcher).map_err(|error| {
        DaemonError::config_invalid(format!(
            "register production MCP tools: {}: {} (remediation: {})",
            error.code, error.message, error.remediation
        ))
    })?;
    Ok(Arc::new(dispatcher))
}

async fn run_servers(
    metrics: MetricsServer,
    mcp: Option<CalyxMcpServer>,
    cancel_token: CancellationToken,
) -> Result<(), DaemonError> {
    let (done_tx, done_rx) = mpsc::channel();
    let metrics_token = cancel_token.clone();
    let metrics_join = spawn_listener("calyxd-metrics", done_tx.clone(), move || {
        metrics.run(metrics_token)
    })?;

    let Some(mcp) = mcp else {
        drop(done_tx);
        wait_for_listener_or_cancel(cancel_token.clone(), done_rx).await?;
        cancel_token.cancel();
        join_listener("metrics", metrics_join)??;
        return Ok(());
    };
    let mcp_addr = mcp.local_addr()?;
    let mcp_shutdown = mcp.shutdown_handle()?;
    let mcp_join = spawn_listener("calyxd-mcp", done_tx, move || {
        println!("INFO calyxd MCP serving on {mcp_addr}");
        mcp.run()
    })?;

    wait_for_listener_or_cancel(cancel_token.clone(), done_rx).await?;
    cancel_token.cancel();
    mcp_shutdown.shutdown();

    join_listener("metrics", metrics_join)??;
    join_listener("mcp", mcp_join)??;
    Ok(())
}

async fn wait_for_listener_or_cancel(
    cancel_token: CancellationToken,
    done_rx: mpsc::Receiver<()>,
) -> Result<(), DaemonError> {
    let listener_done = tokio::task::spawn_blocking(move || done_rx.recv());
    tokio::select! {
        _ = cancel_token.cancelled() => Ok(()),
        result = listener_done => result
            .map(|_| ())
            .map_err(|error| DaemonError::health_failed(format!("wait for listener: {error}"))),
    }
}

fn spawn_listener<F>(
    name: &'static str,
    done_tx: mpsc::Sender<()>,
    run: F,
) -> Result<JoinHandle<Result<(), DaemonError>>, DaemonError>
where
    F: FnOnce() -> Result<(), DaemonError> + Send + 'static,
{
    thread::Builder::new()
        .name(name.to_string())
        .spawn(move || {
            let result = run();
            let _ = done_tx.send(());
            result
        })
        .map_err(|error| DaemonError::health_failed(format!("spawn {name} listener: {error}")))
}

fn join_listener(
    name: &str,
    join: JoinHandle<Result<(), DaemonError>>,
) -> Result<Result<(), DaemonError>, DaemonError> {
    join.join()
        .map_err(|_| DaemonError::health_failed(format!("{name} listener thread panicked")))
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

#[cfg(test)]
mod tests {
    use super::*;
    use calyx_core::{MtlsConfig, TlsConfig};
    use rcgen::{CertifiedKey, generate_simple_self_signed};
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_CERT: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn production_mcp_dispatcher_registers_real_tools() {
        let dispatcher = production_mcp_dispatcher().expect("production MCP tools register");
        assert!(
            dispatcher.tool_count() >= 31,
            "expected the full production tool surface, got {}",
            dispatcher.tool_count()
        );
    }

    #[test]
    fn build_mcp_server_returns_none_when_unconfigured() {
        let cfg = config(None, None);
        assert!(build_mcp_server(&cfg).unwrap().is_none());
    }

    #[test]
    fn build_mcp_server_fails_closed_on_partial_config() {
        let missing_bind = config(None, Some(mtls_config("startup-missing-bind")));
        let Err(error) = build_mcp_server(&missing_bind) else {
            panic!("partial MCP config must fail closed when mcp_bind_addr is missing");
        };
        assert_eq!(error.code(), "CALYX_DAEMON_CONFIG_INVALID");
        assert!(error.to_string().contains("mcp_bind_addr"));

        let missing_mtls = config(Some("127.0.0.1:0".parse().unwrap()), None);
        let Err(error) = build_mcp_server(&missing_mtls) else {
            panic!("partial MCP config must fail closed when mcp_mtls is missing");
        };
        assert_eq!(error.code(), "CALYX_TLS_CONFIG_INVALID");
        assert!(error.to_string().contains("mcp_mtls"));
    }

    #[test]
    fn build_mcp_server_binds_explicit_mcp_addr() {
        let cfg = config(
            Some("127.0.0.1:0".parse().unwrap()),
            Some(mtls_config("startup-bind")),
        );
        let server = build_mcp_server(&cfg)
            .expect("build MCP server")
            .expect("MCP configured");
        let addr = server.local_addr().unwrap();
        assert!(addr.ip().is_loopback());
        assert_ne!(addr.port(), 0);
    }

    fn config(
        mcp_bind_addr: Option<std::net::SocketAddr>,
        mcp_mtls: Option<MtlsConfig>,
    ) -> CalyxConfig {
        CalyxConfig {
            bind_addr: "127.0.0.1:7700".parse().unwrap(),
            mcp_bind_addr,
            vault_path: "/v".into(),
            vram_budget_mib: 8192,
            log_dir: "/l".into(),
            health_log_path: "/h".into(),
            tei_endpoints: Vec::new(),
            healthcheck_timeout_secs: 30,
            max_metrics_connections: 128,
            max_mcp_connections: 128,
            mcp_mtls,
            learner_origin: None,
        }
    }

    fn mtls_config(name: &str) -> MtlsConfig {
        let CertifiedKey { cert, signing_key } =
            generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();
        let id = NEXT_CERT.fetch_add(1, Ordering::SeqCst);
        let root = std::env::temp_dir().join(format!(
            "calyxd-startup-mcp-{name}-{}-{id}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let cert_path = root.join("server-cert.pem");
        let key_path = root.join("server-key.pem");
        let ca_path = root.join("client-ca.pem");
        std::fs::write(&cert_path, cert.pem()).unwrap();
        std::fs::write(&key_path, signing_key.serialize_pem()).unwrap();
        std::fs::write(&ca_path, cert.pem()).unwrap();
        MtlsConfig {
            tls: TlsConfig {
                cert_pem_path: cert_path,
                key_pem_path: key_path,
                ca_pem_path: Some(ca_path),
            },
            require_client_cert: true,
        }
    }
}
