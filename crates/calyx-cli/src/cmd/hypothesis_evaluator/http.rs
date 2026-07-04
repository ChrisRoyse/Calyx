use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

use serde_json::Value;

use super::{evaluator_endpoint_error, evaluator_malformed};
use crate::error::CliResult;

pub(super) fn post_json(endpoint: &str, body: &Value, timeout: Duration) -> CliResult<Value> {
    let parsed = parse_http_url(endpoint)?;
    let addr = (parsed.host.as_str(), parsed.port)
        .to_socket_addrs()
        .map_err(|err| evaluator_endpoint_error(format!("resolve {endpoint}: {err}")))?
        .next()
        .ok_or_else(|| evaluator_endpoint_error(format!("resolve {endpoint}: no addresses")))?;
    let mut stream = TcpStream::connect_timeout(&addr, timeout)
        .map_err(|err| evaluator_endpoint_error(format!("connect {endpoint}: {err}")))?;
    stream
        .set_read_timeout(Some(timeout))
        .map_err(|err| evaluator_endpoint_error(format!("set read timeout: {err}")))?;
    stream
        .set_write_timeout(Some(timeout))
        .map_err(|err| evaluator_endpoint_error(format!("set write timeout: {err}")))?;
    let payload = serde_json::to_vec(body)
        .map_err(|err| evaluator_malformed(format!("serialize evaluator request: {err}")))?;
    let request = format!(
        "POST {} HTTP/1.1\r\nHost: {}\r\nContent-Type: application/json\r\nAccept: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        parsed.path,
        parsed.host_header,
        payload.len()
    );
    stream
        .write_all(request.as_bytes())
        .map_err(|err| evaluator_endpoint_error(format!("write request: {err}")))?;
    stream
        .write_all(&payload)
        .map_err(|err| evaluator_endpoint_error(format!("write request body: {err}")))?;
    let mut response = Vec::new();
    stream
        .read_to_end(&mut response)
        .map_err(|err| evaluator_endpoint_error(format!("read response: {err}")))?;
    parse_http_response(&response)
}

struct ParsedUrl {
    host: String,
    host_header: String,
    port: u16,
    path: String,
}

fn parse_http_url(endpoint: &str) -> CliResult<ParsedUrl> {
    let rest = endpoint.strip_prefix("http://").ok_or_else(|| {
        evaluator_endpoint_error("only http:// evaluator endpoints are supported")
    })?;
    let (authority, path) = match rest.split_once('/') {
        Some((host, path)) => (host, format!("/{path}")),
        None => (rest, "/".to_string()),
    };
    let (host, port) = authority.rsplit_once(':').ok_or_else(|| {
        evaluator_endpoint_error("evaluator endpoint must include explicit host:port")
    })?;
    if host.trim().is_empty() {
        return Err(evaluator_endpoint_error("evaluator endpoint host is empty").into());
    }
    let port = port
        .parse::<u16>()
        .map_err(|err| evaluator_endpoint_error(format!("parse evaluator endpoint port: {err}")))?;
    Ok(ParsedUrl {
        host: host.to_string(),
        host_header: authority.to_string(),
        port,
        path,
    })
}

fn parse_http_response(response: &[u8]) -> CliResult<Value> {
    let marker = b"\r\n\r\n";
    let split = response
        .windows(marker.len())
        .position(|window| window == marker)
        .ok_or_else(|| evaluator_malformed("endpoint response missing HTTP header terminator"))?;
    let headers = String::from_utf8_lossy(&response[..split]);
    let status = headers
        .lines()
        .next()
        .ok_or_else(|| evaluator_malformed("endpoint response missing status line"))?;
    if !status.contains(" 200 ") {
        return Err(
            evaluator_endpoint_error(format!("endpoint returned non-200 status {status}")).into(),
        );
    }
    let body = &response[split + marker.len()..];
    serde_json::from_slice(body)
        .map_err(|err| evaluator_malformed(format!("parse endpoint JSON response: {err}")).into())
}
