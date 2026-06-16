use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::path::Path;
use std::time::{Duration, Instant};

use calyx_aster::cf::ColumnFamily;
use calyx_aster::manifest::ManifestStore;
use calyx_core::{CalyxError, VaultStore};
use serde::Serialize;

use super::value;
use super::vault::{home_dir, resolve_vault_info, vault_salt};
use crate::error::{CliError, CliResult};
use crate::output::print_json;

const TEI_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Clone, Debug, PartialEq, Eq)]
struct Args {
    vault: Option<String>,
    json: bool,
    tei: Vec<Endpoint>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Endpoint {
    name: String,
    host: String,
    port: u16,
    path: String,
}

#[derive(Debug, Serialize)]
struct HealthReport {
    status: &'static str,
    checks: Vec<HealthCheck>,
}

#[derive(Debug, Serialize)]
struct HealthCheck {
    name: String,
    status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    code: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    latency_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    n_cx: Option<usize>,
}

impl Endpoint {
    fn new(
        name: impl Into<String>,
        host: impl Into<String>,
        port: u16,
        path: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            host: host.into(),
            port,
            path: path.into(),
        }
    }
}

pub(crate) fn try_run(args: &[String]) -> Option<CliResult> {
    if args.first().map(String::as_str) != Some("healthcheck") {
        return None;
    }
    if !owns_form(&args[1..]) {
        return None;
    }
    Some(parse(&args[1..]).and_then(run))
}

fn owns_form(rest: &[String]) -> bool {
    if rest.is_empty() {
        return true;
    }
    let mut idx = 0;
    while idx < rest.len() {
        match rest[idx].as_str() {
            "--json" | "--no-json" => idx += 1,
            "--tei" if idx + 1 < rest.len() => idx += 2,
            "--tei" => return true,
            "--vault" if idx + 1 < rest.len() => idx += 2,
            "--vault" => return true,
            _ => return false,
        }
    }
    true
}

fn parse(rest: &[String]) -> CliResult<Args> {
    let mut args = Args {
        vault: None,
        json: true,
        tei: Vec::new(),
    };
    let mut format_seen = false;
    let mut idx = 0;
    while idx < rest.len() {
        match rest[idx].as_str() {
            "--vault" => {
                idx += 1;
                args.vault = Some(
                    rest.get(idx)
                        .ok_or_else(|| CliError::usage("--vault requires a value"))?
                        .clone(),
                );
            }
            "--tei" => {
                idx += 1;
                args.tei.push(parse_endpoint(value(rest, idx, "--tei")?)?);
            }
            "--json" => {
                set_format(&mut format_seen)?;
                args.json = true;
            }
            "--no-json" => {
                set_format(&mut format_seen)?;
                args.json = false;
            }
            other => {
                return Err(CliError::usage(format!(
                    "unexpected healthcheck flag {other}"
                )));
            }
        }
        idx += 1;
    }
    Ok(args)
}

fn run(args: Args) -> CliResult {
    let endpoints = if args.tei.is_empty() {
        default_endpoints()
    } else {
        args.tei.clone()
    };
    let report = build_report(&args, &endpoints);
    if args.json {
        print_json(&report)?;
    } else {
        print_human(&report);
    }
    match first_failure(&report) {
        Some(error) => Err(error.into()),
        None => Ok(()),
    }
}

fn build_report(args: &Args, endpoints: &[Endpoint]) -> HealthReport {
    let mut checks = vec![check_engine()];
    checks.extend(endpoints.iter().map(check_tei));
    if let Some(vault) = &args.vault {
        checks.push(check_vault(vault));
    }
    let status = if checks.iter().all(|check| check.status == "pass") {
        "pass"
    } else {
        "fail"
    };
    HealthReport { status, checks }
}

fn default_endpoints() -> Vec<Endpoint> {
    vec![
        Endpoint::new("tei:8088", "127.0.0.1", 8088, "/"),
        Endpoint::new("tei:8089", "127.0.0.1", 8089, "/"),
        Endpoint::new("tei:8090", "127.0.0.1", 8090, "/"),
    ]
}

fn check_engine() -> HealthCheck {
    match std::env::current_exe() {
        Ok(path) if path.is_file() => pass("engine", None, None),
        Ok(path) => fail(
            "engine",
            CalyxError::forge_device_unavailable(format!(
                "current executable {} is not a file",
                path.display()
            )),
        ),
        Err(error) => fail(
            "engine",
            CalyxError::forge_device_unavailable(format!(
                "current executable unavailable: {error}"
            )),
        ),
    }
}

fn check_tei(endpoint: &Endpoint) -> HealthCheck {
    let started = Instant::now();
    match http_get(endpoint, TEI_TIMEOUT) {
        Ok(()) => pass(
            endpoint.name.clone(),
            Some(started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64),
            None,
        ),
        Err(message) => fail(endpoint.name.clone(), CalyxError::lens_unreachable(message)),
    }
}

fn check_vault(vault: &str) -> HealthCheck {
    match vault_count(vault) {
        Ok(n_cx) if n_cx > 0 => pass("vault", None, Some(n_cx)),
        Ok(_) => fail(
            "vault",
            CalyxError::stale_derived("vault has no base CF rows"),
        ),
        Err(error) => fail("vault", error),
    }
}

fn vault_count(vault: &str) -> Result<usize, CalyxError> {
    let home = home_dir().map_err(|error| CalyxError::vault_access_denied(error.to_string()))?;
    let resolved = resolve_vault_info(&home, vault)
        .map_err(|error| CalyxError::vault_access_denied(error.to_string()))?;
    ensure_manifest(&resolved.path)?;
    let store = calyx_aster::vault::AsterVault::open(
        &resolved.path,
        resolved.vault_id,
        vault_salt(resolved.vault_id, &resolved.name),
        calyx_aster::vault::VaultOptions::default(),
    )?;
    let rows = store.scan_cf_at(store.snapshot(), ColumnFamily::Base)?;
    Ok(rows.len())
}

fn ensure_manifest(vault: &Path) -> Result<(), CalyxError> {
    if !vault.join("CURRENT").is_file() {
        return Err(CalyxError::aster_corrupt_shard(
            "vault CURRENT manifest pointer is missing",
        ));
    }
    ManifestStore::open(vault).load_current()?;
    Ok(())
}

fn http_get(endpoint: &Endpoint, timeout: Duration) -> Result<(), String> {
    let addr = (endpoint.host.as_str(), endpoint.port)
        .to_socket_addrs()
        .map_err(|error| format!("resolve {}:{}: {error}", endpoint.host, endpoint.port))?
        .next()
        .ok_or_else(|| {
            format!(
                "resolve {}:{} returned no address",
                endpoint.host, endpoint.port
            )
        })?;
    let mut stream = TcpStream::connect_timeout(&addr, timeout)
        .map_err(|error| format!("connect {}:{}: {error}", endpoint.host, endpoint.port))?;
    stream
        .set_read_timeout(Some(timeout))
        .map_err(|error| format!("set read timeout: {error}"))?;
    stream
        .set_write_timeout(Some(timeout))
        .map_err(|error| format!("set write timeout: {error}"))?;
    let request = format!(
        "GET {} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
        endpoint.path, endpoint.host
    );
    stream
        .write_all(request.as_bytes())
        .map_err(|error| format!("write request: {error}"))?;
    let mut buf = [0_u8; 64];
    let n = stream
        .read(&mut buf)
        .map_err(|error| format!("read response: {error}"))?;
    if n == 0 || !buf[..n].starts_with(b"HTTP/") {
        return Err(format!(
            "{}:{} did not return an HTTP response",
            endpoint.host, endpoint.port
        ));
    }
    Ok(())
}

fn parse_endpoint(raw: &str) -> CliResult<Endpoint> {
    if raw.is_empty() {
        return Err(CliError::usage(
            "--tei requires host:port or http://host:port[/path]",
        ));
    }
    if raw.starts_with("https://") {
        return Err(CliError::usage(
            "--tei probes plain HTTP; use http://host:port[/path]",
        ));
    }
    let without_scheme = raw.strip_prefix("http://").unwrap_or(raw);
    let (authority, path) = without_scheme
        .split_once('/')
        .map(|(left, right)| (left, format!("/{right}")))
        .unwrap_or((without_scheme, "/".to_string()));
    let (host, port) = authority
        .rsplit_once(':')
        .ok_or_else(|| CliError::usage("--tei requires host:port"))?;
    if host.is_empty() {
        return Err(CliError::usage("--tei host must not be empty"));
    }
    let port = port
        .parse::<u16>()
        .map_err(|error| CliError::usage(format!("parse --tei port: {error}")))?;
    if port == 0 {
        return Err(CliError::usage("--tei port must be greater than zero"));
    }
    let name = if host == "127.0.0.1" || host == "localhost" {
        format!("tei:{port}")
    } else {
        format!("tei:{host}:{port}")
    };
    Ok(Endpoint::new(name, host, port, path))
}

fn set_format(seen: &mut bool) -> CliResult {
    if std::mem::replace(seen, true) {
        Err(CliError::usage("use only one of --json or --no-json"))
    } else {
        Ok(())
    }
}

fn print_human(report: &HealthReport) {
    for check in &report.checks {
        if check.status == "pass" {
            println!("PASS {}", check.name);
        } else {
            println!("FAIL {}", check.name);
        }
    }
}

fn pass(name: impl Into<String>, latency_ms: Option<u64>, n_cx: Option<usize>) -> HealthCheck {
    HealthCheck {
        name: name.into(),
        status: "pass",
        code: None,
        message: None,
        latency_ms,
        n_cx,
    }
}

fn fail(name: impl Into<String>, error: CalyxError) -> HealthCheck {
    HealthCheck {
        name: name.into(),
        status: "fail",
        code: Some(error.code),
        message: Some(error.message),
        latency_ms: None,
        n_cx: None,
    }
}

fn first_failure(report: &HealthReport) -> Option<CalyxError> {
    report
        .checks
        .iter()
        .find(|check| check.status == "fail")
        .map(|check| match check.code {
            Some("CALYX_LENS_UNREACHABLE") => {
                CalyxError::lens_unreachable(check.message.clone().unwrap_or_default())
            }
            Some("CALYX_ASTER_CORRUPT_SHARD") => {
                CalyxError::aster_corrupt_shard(check.message.clone().unwrap_or_default())
            }
            Some("CALYX_STALE_DERIVED") => {
                CalyxError::stale_derived(check.message.clone().unwrap_or_default())
            }
            Some("CALYX_VAULT_ACCESS_DENIED") => {
                CalyxError::vault_access_denied(check.message.clone().unwrap_or_default())
            }
            _ => CalyxError::forge_device_unavailable(
                check.message.clone().unwrap_or_else(|| check.name.clone()),
            ),
        })
}

#[cfg(test)]
mod tests;
