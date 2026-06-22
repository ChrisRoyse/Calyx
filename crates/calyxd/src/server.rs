//! Loopback-only HTTP listener serving `GET /metrics` and optional origin APIs.
//!
//! Binding any non-loopback address is a hard `CALYX_DAEMON_BIND_FAILED`; the
//! daemon does not start. The handler speaks just enough HTTP/1.1 for a
//! Prometheus scrape compatibility is preserved: `GET /metrics` returns text
//! format v0.0.4. When configured, issue #813 Worker-origin routes are also
//! served on the same loopback listener with bounded JSON bodies and bearer auth.

mod http;

use std::io::ErrorKind;
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use tokio_util::sync::CancellationToken;

use crate::error::DaemonError;
use crate::learner_origin::LearnerOriginService;
use crate::metrics::CalyxMetrics;
use http::{
    DEFAULT_BODY_LIMIT, HttpRequest, HttpResponse, IO_TIMEOUT, read_request, write_response,
};

const ACCEPT_IDLE_SLEEP: Duration = Duration::from_millis(10);
const DRAIN_TIMEOUT: Duration = Duration::from_secs(5);
const CONTENT_TYPE: &str = "text/plain; version=0.0.4";
const PLAIN_CONTENT_TYPE: &str = "text/plain; charset=utf-8";
const JSON_CONTENT_TYPE: &str = "application/json";

/// Loopback `/metrics` server.
pub struct MetricsServer {
    listener: TcpListener,
    metrics: Arc<CalyxMetrics>,
    origin: Option<Arc<LearnerOriginService>>,
    active: Arc<AtomicUsize>,
}

impl MetricsServer {
    /// Binds `addr`, refusing any non-loopback IP before touching the OS.
    pub fn bind(addr: SocketAddr, metrics: Arc<CalyxMetrics>) -> Result<Self, DaemonError> {
        Self::bind_inner(addr, metrics, None)
    }

    pub fn bind_with_origin(
        addr: SocketAddr,
        metrics: Arc<CalyxMetrics>,
        origin: Arc<LearnerOriginService>,
    ) -> Result<Self, DaemonError> {
        Self::bind_inner(addr, metrics, Some(origin))
    }

    fn bind_inner(
        addr: SocketAddr,
        metrics: Arc<CalyxMetrics>,
        origin: Option<Arc<LearnerOriginService>>,
    ) -> Result<Self, DaemonError> {
        if !addr.ip().is_loopback() {
            return Err(DaemonError::bind_failed(format!(
                "refused non-loopback bind address {addr}; calyxd serves loopback only"
            )));
        }
        let listener = TcpListener::bind(addr)
            .map_err(|error| DaemonError::bind_failed(format!("bind {addr}: {error}")))?;
        Ok(Self {
            listener,
            metrics,
            origin,
            active: Arc::new(AtomicUsize::new(0)),
        })
    }

    /// The actually-bound address (port 0 resolves here).
    pub fn local_addr(&self) -> Result<SocketAddr, DaemonError> {
        self.listener
            .local_addr()
            .map_err(|error| DaemonError::bind_failed(format!("local_addr: {error}")))
    }

    /// Number of connection handlers currently in flight.
    pub fn active_connections(&self) -> usize {
        self.active.load(Ordering::SeqCst)
    }

    /// Accept loop; each connection is served on its own thread so one stuck
    /// client cannot block the next scrape. The loop returns only after
    /// `cancel_token` fires and in-flight handlers have drained or timed out.
    pub fn run(self, cancel_token: CancellationToken) -> Result<(), DaemonError> {
        self.listener.set_nonblocking(true).map_err(|error| {
            DaemonError::bind_failed(format!("set metrics listener nonblocking: {error}"))
        })?;
        while !cancel_token.is_cancelled() {
            match self.listener.accept() {
                Ok((stream, peer)) => {
                    let metrics = Arc::clone(&self.metrics);
                    let origin = self.origin.as_ref().map(Arc::clone);
                    let active = Arc::clone(&self.active);
                    active.fetch_add(1, Ordering::SeqCst);
                    std::thread::spawn(move || {
                        let outcome = catch_unwind(AssertUnwindSafe(|| {
                            handle_connection(stream, &metrics, origin.as_deref())
                        }));
                        active.fetch_sub(1, Ordering::SeqCst);
                        match outcome {
                            Ok(Ok(())) => {}
                            Ok(Err(detail)) => {
                                eprintln!("calyxd: metrics connection from {peer}: {detail}");
                            }
                            Err(_panic) => {
                                eprintln!(
                                    "calyxd: CALYX_DAEMON_CONN_PANIC: metrics connection from \
                                     {peer} panicked; connection dropped, server continues"
                                );
                            }
                        }
                    });
                }
                Err(error) if error.kind() == ErrorKind::WouldBlock => {
                    std::thread::sleep(ACCEPT_IDLE_SLEEP);
                }
                Err(error) => {
                    eprintln!("calyxd: accept on metrics listener failed: {error}");
                }
            }
        }

        let deadline = Instant::now() + DRAIN_TIMEOUT;
        while self.active.load(Ordering::SeqCst) > 0 && Instant::now() < deadline {
            std::thread::sleep(ACCEPT_IDLE_SLEEP);
        }
        Ok(())
    }
}

/// Serves exactly one HTTP request on `stream`.
fn handle_connection(
    mut stream: TcpStream,
    metrics: &CalyxMetrics,
    origin: Option<&LearnerOriginService>,
) -> Result<(), String> {
    stream
        .set_read_timeout(Some(IO_TIMEOUT))
        .map_err(|error| format!("set read timeout: {error}"))?;
    stream
        .set_write_timeout(Some(IO_TIMEOUT))
        .map_err(|error| format!("set write timeout: {error}"))?;

    let max_body = origin
        .map(LearnerOriginService::max_body_bytes)
        .unwrap_or(DEFAULT_BODY_LIMIT);
    let request = match read_request(&mut stream, max_body) {
        Ok(request) => request,
        Err(error) => {
            let response = HttpResponse {
                status: error.status(),
                content_type: PLAIN_CONTENT_TYPE,
                body: error.body(),
            };
            write_response(&mut stream, &response)?;
            return Err(format!("unreadable request: {error:?}"));
        }
    };

    let response = route(&request, metrics, origin);
    write_response(&mut stream, &response)
}

/// Routes one parsed request to a response.
fn route(
    request: &HttpRequest,
    metrics: &CalyxMetrics,
    origin: Option<&LearnerOriginService>,
) -> HttpResponse {
    if let Some(origin) = origin
        && origin.handles_path(&request.path)
        && request.path != "/metrics"
    {
        let response = origin.handle(
            &request.method,
            &request.path,
            request.header("authorization"),
            &request.body,
        );
        return HttpResponse {
            status: response.status,
            content_type: JSON_CONTENT_TYPE,
            body: response.body,
        };
    }
    match (request.method.as_str(), request.path.as_str()) {
        ("GET", "/metrics") => match metrics.encode_text() {
            Ok(mut text) => {
                if let Some(origin) = origin {
                    match origin.metrics().encode_text() {
                        Ok(origin_text) => text.push_str(&origin_text),
                        Err(detail) => {
                            eprintln!("calyxd: {detail}");
                            return HttpResponse {
                                status: "500 Internal Server Error",
                                content_type: PLAIN_CONTENT_TYPE,
                                body: format!("{detail}\n"),
                            };
                        }
                    }
                }
                HttpResponse {
                    status: "200 OK",
                    content_type: CONTENT_TYPE,
                    body: text,
                }
            }
            Err(detail) => {
                eprintln!("calyxd: {detail}");
                HttpResponse {
                    status: "500 Internal Server Error",
                    content_type: PLAIN_CONTENT_TYPE,
                    body: format!("{detail}\n"),
                }
            }
        },
        ("GET", _) => HttpResponse {
            status: "404 Not Found",
            content_type: PLAIN_CONTENT_TYPE,
            body: "only /metrics is served\n".to_string(),
        },
        _ => HttpResponse {
            status: "405 Method Not Allowed",
            content_type: PLAIN_CONTENT_TYPE,
            body: "only GET /metrics is served unless learner_origin is configured\n".to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::ChainVerifyMetrics;
    use std::io::{Read, Write};

    fn metrics() -> Arc<CalyxMetrics> {
        let labels = ["/tmp/vault".to_string()];
        let chain = Arc::new(ChainVerifyMetrics::new(&labels));
        Arc::new(CalyxMetrics::new(chain, &labels))
    }

    fn request(line: &str) -> HttpRequest {
        let mut parts = line.split_whitespace();
        HttpRequest {
            method: parts.next().unwrap().to_string(),
            path: parts.next().unwrap().to_string(),
            headers: Vec::new(),
            body: Vec::new(),
        }
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
    fn route_serves_full_metric_surface() {
        let metrics = metrics();
        let response = route(&request("GET /metrics HTTP/1.1"), &metrics, None);
        assert_eq!(response.status, "200 OK");
        // Chain-verify family (issue #602) plus the PH66 T03 families and the
        // 25 hazard gauges are all served from the one route.
        assert!(response.body.contains("calyx_ledger_chain_verify_ok"));
        assert!(response.body.contains("calyx_ingest_duration_seconds"));
        assert!(response.body.contains("calyx_search_recall_tripwire"));
        assert!(response.body.contains("calyx_vram_budget_limit_mib"));
        assert!(response.body.contains("calyx_zfs_pool_healthy"));
        let hazard_lines = response
            .body
            .lines()
            .filter(|line| line.starts_with("calyx_hazard_"))
            .count();
        assert_eq!(hazard_lines, 25);
    }

    #[test]
    fn metrics_response_uses_prometheus_content_type() {
        // The exposition format version is mandatory for Prometheus to parse the
        // body; assert the exact header value the handler writes.
        assert_eq!(CONTENT_TYPE, "text/plain; version=0.0.4");
    }

    #[test]
    fn route_rejects_unknown_path_and_method() {
        let metrics = metrics();
        assert_eq!(
            route(&request("GET /health HTTP/1.1"), &metrics, None).status,
            "404 Not Found"
        );
        assert_eq!(
            route(&request("POST /metrics HTTP/1.1"), &metrics, None).status,
            "405 Method Not Allowed"
        );
    }

    #[test]
    fn cancellation_token_stops_accept_loop() {
        let server = MetricsServer::bind("127.0.0.1:0".parse().unwrap(), metrics()).unwrap();
        let addr = server.local_addr().unwrap();
        assert_eq!(server.active_connections(), 0);
        let token = CancellationToken::new();
        let stop = token.clone();
        let join = std::thread::spawn(move || server.run(token));

        let mut stream = TcpStream::connect(addr).expect("connect before shutdown");
        write!(stream, "GET /metrics HTTP/1.1\r\nHost: {addr}\r\n\r\n").expect("send request");
        let mut response = String::new();
        stream.read_to_string(&mut response).expect("read response");
        assert!(response.starts_with("HTTP/1.1 200 OK"), "{response}");

        stop.cancel();
        join.join()
            .expect("server thread joins")
            .expect("server returns Ok");
    }
}
