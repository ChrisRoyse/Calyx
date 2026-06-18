//! `calyx healthcheck` writes the deploy health source-of-truth JSON.
//!
//! The command is intentionally file-backed: it probes the rendered secret
//! env, the Calyx home, and any requested vault/metrics source, then writes
//! `latest.json` and reads it back before returning.

use std::collections::BTreeSet;
use std::fs;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use calyx_core::CalyxError;
use calyxd::verify::verify_restore;
use serde::Serialize;

const DEFAULT_OUT: &str = "/zfs/hot/logs/calyx-health/latest.json";
const DEFAULT_SECRET_ENV: &str = "/run/leapable/secrets/calyx.env";
const DEFAULT_CALYX_HOME: &str = "/var/lib/calyx";
const DEFAULT_REQUIRED_ENV: [&str; 2] = ["HF_HUB_TOKEN", "HF_TOKEN"];
const CALYX_HEALTHCHECK_FAILED: &str = "CALYX_HEALTHCHECK_FAILED";
const HEALTHCHECK_FAILED_REMEDIATION: &str =
    "inspect the written healthcheck JSON source of truth and fix failed checks";

#[derive(Debug)]
struct HealthArgs {
    wait_secs: u64,
    out: PathBuf,
    secret_env: PathBuf,
    calyx_home: PathBuf,
    required_env: Vec<String>,
    vault: Option<PathBuf>,
    metrics_url: Option<String>,
}

#[derive(Debug, Serialize)]
struct HealthReport {
    run_id: String,
    source_of_truth: Vec<String>,
    checked_at_unix_secs: u64,
    status: &'static str,
    failure_count: usize,
    checks: Vec<HealthCheck>,
}

#[derive(Debug, Serialize)]
struct HealthCheck {
    name: &'static str,
    status: &'static str,
    code: Option<&'static str>,
    detail: String,
}

pub(crate) fn run(args: &[String]) -> crate::error::CliResult {
    let request = HealthArgs::parse(args)?;
    let started = Instant::now();
    let wait = Duration::from_secs(request.wait_secs);

    loop {
        let report = build_report(&request);
        write_and_read_back(&request.out, &report)?;
        if report.failure_count == 0 {
            return Ok(());
        }
        let failure = failure_error(&request.out, &report);
        if started.elapsed() >= wait {
            return Err(failure.into());
        }
        thread::sleep(Duration::from_secs(1));
    }
}

impl HealthArgs {
    fn parse(args: &[String]) -> Result<Self, String> {
        let mut request = Self {
            wait_secs: 0,
            out: env_path("CALYX_HEALTH_LOG_PATH").unwrap_or_else(|| PathBuf::from(DEFAULT_OUT)),
            secret_env: env_path("CALYX_SECRET_ENV")
                .unwrap_or_else(|| PathBuf::from(DEFAULT_SECRET_ENV)),
            calyx_home: env_path("CALYX_HOME").unwrap_or_else(|| PathBuf::from(DEFAULT_CALYX_HOME)),
            required_env: DEFAULT_REQUIRED_ENV
                .iter()
                .map(|name| name.to_string())
                .collect(),
            vault: env_path("CALYX_HEALTH_VAULT"),
            metrics_url: std::env::var("CALYX_HEALTH_METRICS_URL").ok(),
        };

        let mut i = 0;
        while i < args.len() {
            match args[i].as_str() {
                "--wait" => {
                    request.wait_secs = parse_u64("--wait", value(args, i)?)?;
                    i += 2;
                }
                "--out" => {
                    request.out = PathBuf::from(value(args, i)?);
                    i += 2;
                }
                "--secret-env" => {
                    request.secret_env = PathBuf::from(value(args, i)?);
                    i += 2;
                }
                "--calyx-home" => {
                    request.calyx_home = PathBuf::from(value(args, i)?);
                    i += 2;
                }
                "--vault" => {
                    request.vault = Some(PathBuf::from(value(args, i)?));
                    i += 2;
                }
                "--metrics-url" => {
                    request.metrics_url = Some(value(args, i)?.to_string());
                    i += 2;
                }
                "--require-env" => {
                    request.required_env.push(value(args, i)?.to_string());
                    i += 2;
                }
                other => {
                    return Err(format!("CALYX_HEALTH_CONFIG_INVALID: unknown arg {other}"));
                }
            }
        }
        if request.required_env.iter().any(|name| name.is_empty()) {
            return Err("CALYX_HEALTH_CONFIG_INVALID: empty --require-env".to_string());
        }
        Ok(request)
    }
}

fn build_report(request: &HealthArgs) -> HealthReport {
    let mut checks = vec![
        check_calyx_home(&request.calyx_home),
        check_secret_env(&request.secret_env, &request.required_env),
    ];
    if let Some(vault) = &request.vault {
        checks.push(check_vault(vault));
    }
    if let Some(url) = &request.metrics_url {
        checks.push(check_metrics(url));
    }
    let failure_count = checks.iter().filter(|check| check.status != "pass").count();
    HealthReport {
        run_id: format!("calyx-health-{}-{}", unix_secs(), std::process::id()),
        source_of_truth: source_of_truth(request),
        checked_at_unix_secs: unix_secs(),
        status: if failure_count == 0 { "pass" } else { "fail" },
        failure_count,
        checks,
    }
}

fn source_of_truth(request: &HealthArgs) -> Vec<String> {
    let mut sources = vec![
        request.out.display().to_string(),
        request.secret_env.display().to_string(),
        request.calyx_home.display().to_string(),
    ];
    if let Some(vault) = &request.vault {
        sources.push(vault.display().to_string());
    }
    if let Some(url) = &request.metrics_url {
        sources.push(url.clone());
    }
    sources
}

fn check_calyx_home(home: &Path) -> HealthCheck {
    if !home.is_dir() {
        return fail(
            "calyx_home",
            "CALYX_HEALTH_HOME_MISSING",
            format!("{} is not a directory", home.display()),
        );
    }
    if !home.join("repo").is_dir() {
        return fail(
            "calyx_repo",
            "CALYX_HEALTH_HOME_MISSING",
            format!("{} has no repo/ checkout", home.display()),
        );
    }
    pass(
        "calyx_home",
        format!("{} exists with repo/", home.display()),
    )
}

fn check_secret_env(path: &Path, required_env: &[String]) -> HealthCheck {
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) => {
            return fail(
                "calyx_secret_env",
                "CALYX_HEALTH_SECRET_ENV_MISSING",
                format!("{}: {error}", path.display()),
            );
        }
    };
    if !metadata.is_file() {
        return fail(
            "calyx_secret_env",
            "CALYX_HEALTH_SECRET_ENV_MISSING",
            format!("{} is not a file", path.display()),
        );
    }
    if let Err(detail) = check_mode_0400(path, &metadata) {
        return fail(
            "calyx_secret_env_mode",
            "CALYX_HEALTH_SECRET_ENV_MODE",
            detail,
        );
    }
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) => {
            return fail(
                "calyx_secret_env",
                "CALYX_HEALTH_SECRET_ENV_READ",
                format!("{}: {error}", path.display()),
            );
        }
    };
    let present = env_names(&text);
    for name in required_env {
        if !present.contains(name) {
            return fail(
                "calyx_secret_env_vars",
                "CALYX_HEALTH_SECRET_ENV_VAR_MISSING",
                format!("{} lacks required env var {name}", path.display()),
            );
        }
    }
    pass(
        "calyx_secret_env",
        format!(
            "{} mode=0400 contains required env var names {:?}",
            path.display(),
            required_env
        ),
    )
}

fn check_vault(vault: &Path) -> HealthCheck {
    match verify_restore(vault) {
        Ok(report) if report.success() => pass(
            "calyx_vault_restore_readback",
            format!(
                "constellations={} anchors={} ledger_entries={} wal_bytes={}",
                report.constellation_count,
                report.anchor_count,
                report.ledger_entry_count,
                report.wal_bytes_present
            ),
        ),
        Ok(report) => fail(
            "calyx_vault_restore_readback",
            "CALYX_HEALTH_VAULT_UNVERIFIED",
            report.failure_reasons().join("; "),
        ),
        Err(error) => fail(
            "calyx_vault_restore_readback",
            "CALYX_HEALTH_VAULT_UNVERIFIED",
            error.to_string(),
        ),
    }
}

fn check_metrics(url: &str) -> HealthCheck {
    match scrape_metrics(url) {
        Ok(body) if metrics_have_verified_target(&body) => pass(
            "calyx_metrics",
            format!("{url} contains calyx_ledger_chain_verify_ok == 1"),
        ),
        Ok(body) if body.contains("calyx_ledger_chain_verify_ok") => fail(
            "calyx_metrics",
            "CALYX_HEALTH_METRICS_FAILING",
            format!("{url} has no verified chain target"),
        ),
        Ok(_) => fail(
            "calyx_metrics",
            "CALYX_HEALTH_METRICS_MISSING",
            format!("{url} lacks calyx_ledger_chain_verify_ok"),
        ),
        Err(error) => fail("calyx_metrics", "CALYX_HEALTH_METRICS_UNREACHABLE", error),
    }
}

fn write_and_read_back(path: &Path, report: &HealthReport) -> std::result::Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "CALYX_HEALTH_WRITEBACK: create {}: {error}",
                parent.display()
            )
        })?;
    }
    let json = serde_json::to_string_pretty(report).map_err(|error| error.to_string())?;
    fs::write(path, format!("{json}\n"))
        .map_err(|error| format!("CALYX_HEALTH_WRITEBACK: write {}: {error}", path.display()))?;
    let readback = fs::read_to_string(path)
        .map_err(|error| format!("CALYX_HEALTH_WRITEBACK: read {}: {error}", path.display()))?;
    let value: serde_json::Value = serde_json::from_str(&readback)
        .map_err(|error| format!("CALYX_HEALTH_WRITEBACK: parse readback: {error}"))?;
    let status = value
        .get("status")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "CALYX_HEALTH_WRITEBACK: readback missing status".to_string())?;
    if status != report.status {
        return Err(format!(
            "CALYX_HEALTH_WRITEBACK: status mismatch wrote={} read={status}",
            report.status
        ));
    }
    Ok(())
}

fn scrape_metrics(url: &str) -> Result<String, String> {
    let parsed = ParsedHttpUrl::parse(url)?;
    let mut stream = TcpStream::connect((&*parsed.host, parsed.port))
        .map_err(|error| format!("connect {url}: {error}"))?;
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .map_err(|error| format!("set read timeout: {error}"))?;
    stream
        .set_write_timeout(Some(Duration::from_secs(5)))
        .map_err(|error| format!("set write timeout: {error}"))?;
    let request = format!(
        "GET {} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
        parsed.path, parsed.host
    );
    stream
        .write_all(request.as_bytes())
        .map_err(|error| format!("write request: {error}"))?;
    let mut response = String::new();
    stream
        .read_to_string(&mut response)
        .map_err(|error| format!("read response: {error}"))?;
    if !response.starts_with("HTTP/1.1 200") && !response.starts_with("HTTP/1.0 200") {
        let status = response.lines().next().unwrap_or("empty response");
        return Err(format!("{url} returned {status}"));
    }
    Ok(response)
}

fn metrics_have_verified_target(body: &str) -> bool {
    body.lines()
        .filter(|line| line.starts_with("calyx_ledger_chain_verify_ok"))
        .any(|line| line.split_whitespace().last() == Some("1"))
}

struct ParsedHttpUrl {
    host: String,
    port: u16,
    path: String,
}

impl ParsedHttpUrl {
    fn parse(url: &str) -> Result<Self, String> {
        let rest = url.strip_prefix("http://").ok_or_else(|| {
            "CALYX_HEALTH_CONFIG_INVALID: metrics URL must use http://".to_string()
        })?;
        let (authority, path) = match rest.split_once('/') {
            Some((authority, path)) => (authority, format!("/{path}")),
            None => (rest, "/".to_string()),
        };
        if authority.is_empty() {
            return Err("CALYX_HEALTH_CONFIG_INVALID: metrics URL host is empty".to_string());
        }
        let (host, port) = match authority.rsplit_once(':') {
            Some((host, port)) => (
                host.to_string(),
                port.parse::<u16>().map_err(|error| {
                    format!("CALYX_HEALTH_CONFIG_INVALID: metrics URL port: {error}")
                })?,
            ),
            None => (authority.to_string(), 80),
        };
        Ok(Self { host, port, path })
    }
}

#[cfg(unix)]
fn check_mode_0400(path: &Path, metadata: &fs::Metadata) -> std::result::Result<(), String> {
    use std::os::unix::fs::PermissionsExt;

    let mode = metadata.permissions().mode() & 0o777;
    if mode == 0o400 {
        Ok(())
    } else {
        Err(format!(
            "{} mode is {:o}, expected 400",
            path.display(),
            mode
        ))
    }
}

#[cfg(not(unix))]
fn check_mode_0400(_path: &Path, _metadata: &fs::Metadata) -> std::result::Result<(), String> {
    Ok(())
}

fn env_names(text: &str) -> BTreeSet<String> {
    text.lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                return None;
            }
            trimmed
                .split_once('=')
                .map(|(name, _)| name.trim().to_string())
        })
        .collect()
}

fn value(args: &[String], index: usize) -> Result<&str, String> {
    args.get(index + 1).map(String::as_str).ok_or_else(|| {
        format!(
            "CALYX_HEALTH_CONFIG_INVALID: {} requires a value",
            args[index]
        )
    })
}

fn parse_u64(flag: &str, value: &str) -> Result<u64, String> {
    value
        .parse()
        .map_err(|error| format!("CALYX_HEALTH_CONFIG_INVALID: {flag} {value}: {error}"))
}

fn env_path(name: &str) -> Option<PathBuf> {
    std::env::var_os(name).map(PathBuf::from)
}

fn pass(name: &'static str, detail: String) -> HealthCheck {
    HealthCheck {
        name,
        status: "pass",
        code: None,
        detail,
    }
}

fn fail(name: &'static str, code: &'static str, detail: String) -> HealthCheck {
    HealthCheck {
        name,
        status: "fail",
        code: Some(code),
        detail,
    }
}

fn failure_error(path: &Path, report: &HealthReport) -> CalyxError {
    let details = report
        .checks
        .iter()
        .filter(|check| check.status != "pass")
        .map(|check| {
            format!(
                "{}={}",
                check.name,
                check.code.unwrap_or("CALYX_HEALTH_FAIL")
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    CalyxError {
        code: CALYX_HEALTHCHECK_FAILED,
        message: format!(
            "wrote {} with {} failure(s): {details}",
            path.display(),
            report.failure_count
        ),
        remediation: HEALTHCHECK_FAILED_REMEDIATION,
    }
}

fn unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}
