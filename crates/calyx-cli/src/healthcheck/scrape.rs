//! Plain-HTTP metrics scrape for the deploy healthcheck.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

pub(super) fn scrape_metrics(url: &str) -> Result<String, String> {
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

pub(super) fn metrics_have_verified_target(body: &str) -> bool {
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
