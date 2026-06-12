//! Loopback-only HTTP listener serving `GET /metrics` (PH65 bind rules).
//!
//! Binding any non-loopback address is a hard `CALYX_DAEMON_BIND_FAILED`; the
//! daemon does not start. The handler speaks just enough HTTP/1.1 for a
//! Prometheus scrape: `GET /metrics` returns text format v0.0.4, every other
//! path is 404, every other method is 405, and an unreadable request is 400.
//! Failures are answered with explicit status codes and logged — never dropped.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::Arc;
use std::time::Duration;

use crate::error::DaemonError;
use crate::metrics::ChainVerifyMetrics;

const REQUEST_HEAD_LIMIT: usize = 8192;
const IO_TIMEOUT: Duration = Duration::from_secs(5);
const CONTENT_TYPE: &str = "text/plain; version=0.0.4";

/// Loopback `/metrics` server.
pub struct MetricsServer {
    listener: TcpListener,
    metrics: Arc<ChainVerifyMetrics>,
}

impl MetricsServer {
    /// Binds `addr`, refusing any non-loopback IP before touching the OS.
    pub fn bind(addr: SocketAddr, metrics: Arc<ChainVerifyMetrics>) -> Result<Self, DaemonError> {
        if !addr.ip().is_loopback() {
            return Err(DaemonError::bind_failed(format!(
                "refused non-loopback bind address {addr}; calyxd serves loopback only"
            )));
        }
        let listener = TcpListener::bind(addr)
            .map_err(|error| DaemonError::bind_failed(format!("bind {addr}: {error}")))?;
        Ok(Self { listener, metrics })
    }

    /// The actually-bound address (port 0 resolves here).
    pub fn local_addr(&self) -> Result<SocketAddr, DaemonError> {
        self.listener
            .local_addr()
            .map_err(|error| DaemonError::bind_failed(format!("local_addr: {error}")))
    }

    /// Accept loop; each connection is served on its own thread so one stuck
    /// client cannot block the next scrape.
    pub fn run(self) -> ! {
        loop {
            match self.listener.accept() {
                Ok((stream, peer)) => {
                    let metrics = Arc::clone(&self.metrics);
                    std::thread::spawn(move || {
                        if let Err(detail) = handle_connection(stream, &metrics) {
                            eprintln!("calyxd: metrics connection from {peer}: {detail}");
                        }
                    });
                }
                Err(error) => {
                    eprintln!("calyxd: accept on metrics listener failed: {error}");
                }
            }
        }
    }
}

/// Serves exactly one HTTP request on `stream`.
fn handle_connection(mut stream: TcpStream, metrics: &ChainVerifyMetrics) -> Result<(), String> {
    stream
        .set_read_timeout(Some(IO_TIMEOUT))
        .map_err(|error| format!("set read timeout: {error}"))?;
    stream
        .set_write_timeout(Some(IO_TIMEOUT))
        .map_err(|error| format!("set write timeout: {error}"))?;

    let request_line = match read_request_line(&mut stream) {
        Ok(line) => line,
        Err(detail) => {
            write_response(&mut stream, "400 Bad Request", "bad request\n")?;
            return Err(format!("unreadable request head: {detail}"));
        }
    };

    let (status, body) = route(&request_line, metrics);
    write_response(&mut stream, status, &body)
}

/// Routes one request line to a status + body.
fn route(request_line: &str, metrics: &ChainVerifyMetrics) -> (&'static str, String) {
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default();
    let path = parts.next().unwrap_or_default();
    match (method, path) {
        ("GET", "/metrics") => match metrics.encode_text() {
            Ok(text) => ("200 OK", text),
            Err(detail) => {
                eprintln!("calyxd: {detail}");
                ("500 Internal Server Error", format!("{detail}\n"))
            }
        },
        ("GET", _) => ("404 Not Found", "only /metrics is served\n".to_string()),
        _ => (
            "405 Method Not Allowed",
            "only GET /metrics is served\n".to_string(),
        ),
    }
}

/// Reads the full request head (through the blank line ending the headers),
/// bounded by `REQUEST_HEAD_LIMIT` bytes, and returns the request line.
/// Consuming the whole head matters: closing a socket with unread request
/// bytes sends TCP RST instead of FIN and aborts the scraper's read.
fn read_request_line(stream: &mut TcpStream) -> Result<String, String> {
    let mut head = Vec::new();
    let mut byte = [0_u8; 1];
    while head.len() < REQUEST_HEAD_LIMIT {
        match stream.read(&mut byte) {
            Ok(0) => break,
            Ok(_) => {
                head.push(byte[0]);
                if head.ends_with(b"\r\n\r\n") || head.ends_with(b"\n\n") {
                    break;
                }
            }
            Err(error) => return Err(format!("read: {error}")),
        }
    }
    if head.len() >= REQUEST_HEAD_LIMIT {
        return Err(format!("request head exceeds {REQUEST_HEAD_LIMIT} bytes"));
    }
    let text =
        String::from_utf8(head).map_err(|error| format!("request head is not utf-8: {error}"))?;
    let line = text
        .lines()
        .next()
        .ok_or_else(|| "empty request head".to_string())?;
    Ok(line.to_string())
}

/// Writes a minimal HTTP/1.1 response and closes the connection.
fn write_response(stream: &mut TcpStream, status: &str, body: &str) -> Result<(), String> {
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {CONTENT_TYPE}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream
        .write_all(response.as_bytes())
        .map_err(|error| format!("write response: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn metrics() -> Arc<ChainVerifyMetrics> {
        Arc::new(ChainVerifyMetrics::new(&["/tmp/vault".to_string()]))
    }

    #[test]
    fn bind_refuses_non_loopback_address() {
        let Err(error) = MetricsServer::bind("0.0.0.0:7700".parse().unwrap(), metrics()) else {
            panic!("non-loopback bind must fail");
        };
        assert_eq!(error.code(), "CALYX_DAEMON_BIND_FAILED");
        assert!(error.to_string().contains("0.0.0.0:7700"));
    }

    #[test]
    fn bind_accepts_ipv4_loopback() {
        let server = MetricsServer::bind("127.0.0.1:0".parse().unwrap(), metrics()).unwrap();
        assert!(server.local_addr().unwrap().ip().is_loopback());
    }

    #[test]
    fn route_serves_metrics_text() {
        let metrics = metrics();
        let (status, body) = route("GET /metrics HTTP/1.1", &metrics);
        assert_eq!(status, "200 OK");
        assert!(body.contains("calyx_ledger_chain_verify_ok"));
    }

    #[test]
    fn route_rejects_unknown_path_and_method() {
        let metrics = metrics();
        assert_eq!(route("GET /health HTTP/1.1", &metrics).0, "404 Not Found");
        assert_eq!(
            route("POST /metrics HTTP/1.1", &metrics).0,
            "405 Method Not Allowed"
        );
    }
}
