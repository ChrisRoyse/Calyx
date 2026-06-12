//! Request-scoped reranker hook for the :8089 cross-encoder surface.

use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

use calyx_core::Result;
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::error::{CALYX_SEXTANT_RERANKER_TIMEOUT, sextant_error};

#[derive(Clone, Debug, PartialEq)]
pub struct RerankRequest {
    pub query: String,
    pub candidates: Vec<Zeroizing<String>>,
}

impl RerankRequest {
    pub fn new(query: impl Into<String>, candidates: Vec<String>) -> Self {
        Self {
            query: query.into(),
            candidates: candidates.into_iter().map(Zeroizing::new).collect(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RerankResponse {
    pub scores: Vec<f32>,
    pub zeroizing_ok: bool,
}

#[derive(Debug, Deserialize)]
struct WireRerankResponse {
    scores: Vec<f32>,
    zeroizing_ok: Option<bool>,
}

#[derive(Serialize)]
struct WireRerankRequest<'a> {
    query: &'a str,
    texts: Vec<&'a str>,
}

#[derive(Debug, Deserialize)]
struct WireRank {
    index: usize,
    score: f32,
}

#[derive(Clone, Debug)]
pub struct RerankerClient {
    endpoint: String,
    timeout: Duration,
}

impl RerankerClient {
    pub fn new(endpoint: impl Into<String>, timeout: Duration) -> Self {
        Self {
            endpoint: endpoint.into(),
            timeout,
        }
    }

    pub fn rerank(&self, request: &RerankRequest) -> Result<RerankResponse> {
        if !self.endpoint.starts_with("http://") {
            return Err(sextant_error(
                CALYX_SEXTANT_RERANKER_TIMEOUT,
                "only http endpoints are supported",
            ));
        }
        let without_scheme = &self.endpoint["http://".len()..];
        let host_port = without_scheme.split('/').next().unwrap_or(without_scheme);
        let addr = host_port
            .to_socket_addrs()
            .map_err(|_| sextant_error(CALYX_SEXTANT_RERANKER_TIMEOUT, "bad reranker endpoint"))?
            .next()
            .ok_or_else(|| sextant_error(CALYX_SEXTANT_RERANKER_TIMEOUT, "no reranker addr"))?;
        let mut stream = TcpStream::connect_timeout(&addr, self.timeout)
            .map_err(|_| sextant_error(CALYX_SEXTANT_RERANKER_TIMEOUT, "connect timeout"))?;
        stream
            .set_read_timeout(Some(self.timeout))
            .map_err(|error| {
                sextant_error(
                    CALYX_SEXTANT_RERANKER_TIMEOUT,
                    format!("set reranker read timeout failed: {error}"),
                )
            })?;
        stream
            .set_write_timeout(Some(self.timeout))
            .map_err(|error| {
                sextant_error(
                    CALYX_SEXTANT_RERANKER_TIMEOUT,
                    format!("set reranker write timeout failed: {error}"),
                )
            })?;
        let texts = request
            .candidates
            .iter()
            .map(|candidate| candidate.as_str())
            .collect();
        let wire_request = WireRerankRequest {
            query: &request.query,
            texts,
        };
        let body = Zeroizing::new(serde_json::to_string(&wire_request).map_err(|error| {
            sextant_error(
                CALYX_SEXTANT_RERANKER_TIMEOUT,
                format!("serialize rerank request failed: {error}"),
            )
        })?);
        let http = Zeroizing::new(format!(
            "POST /rerank HTTP/1.1\r\nHost: {host_port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len(),
            body = &*body
        ));
        stream
            .write_all(http.as_bytes())
            .map_err(|_| sextant_error(CALYX_SEXTANT_RERANKER_TIMEOUT, "write timeout"))?;
        let mut response = String::new();
        stream
            .read_to_string(&mut response)
            .map_err(|_| sextant_error(CALYX_SEXTANT_RERANKER_TIMEOUT, "read timeout"))?;
        ensure_success_status(&response)?;
        parse_http_rerank_response(&response, request.candidates.len())
    }
}

fn ensure_success_status(response: &str) -> Result<()> {
    if response.starts_with("HTTP/1.1 2") || response.starts_with("HTTP/1.0 2") {
        return Ok(());
    }
    let status = response.lines().next().unwrap_or("missing HTTP status");
    Err(sextant_error(
        CALYX_SEXTANT_RERANKER_TIMEOUT,
        format!("reranker returned non-2xx status: {status}"),
    ))
}

fn parse_http_rerank_response(response: &str, expected_scores: usize) -> Result<RerankResponse> {
    let body = response.split("\r\n\r\n").nth(1).ok_or_else(|| {
        sextant_error(
            CALYX_SEXTANT_RERANKER_TIMEOUT,
            "reranker response missing HTTP body",
        )
    })?;
    if body.trim_start().starts_with('[') {
        return parse_tei_rank_response(body, expected_scores);
    }
    let wire: WireRerankResponse = serde_json::from_str(body).map_err(|error| {
        sextant_error(
            CALYX_SEXTANT_RERANKER_TIMEOUT,
            format!("invalid reranker JSON: {error}"),
        )
    })?;
    if wire.scores.len() != expected_scores || wire.scores.iter().any(|score| !score.is_finite()) {
        return Err(sextant_error(
            CALYX_SEXTANT_RERANKER_TIMEOUT,
            "reranker returned invalid score vector",
        ));
    }
    Ok(RerankResponse {
        scores: wire.scores,
        zeroizing_ok: wire.zeroizing_ok.unwrap_or(true),
    })
}

fn parse_tei_rank_response(body: &str, expected_scores: usize) -> Result<RerankResponse> {
    let ranks: Vec<WireRank> = serde_json::from_str(body).map_err(|error| {
        sextant_error(
            CALYX_SEXTANT_RERANKER_TIMEOUT,
            format!("invalid reranker JSON: {error}"),
        )
    })?;
    if ranks.len() != expected_scores {
        return Err(sextant_error(
            CALYX_SEXTANT_RERANKER_TIMEOUT,
            "reranker returned invalid rank vector",
        ));
    }
    let mut scores = vec![f32::NAN; expected_scores];
    for rank in ranks {
        if rank.index >= expected_scores
            || !rank.score.is_finite()
            || scores[rank.index].is_finite()
        {
            return Err(sextant_error(
                CALYX_SEXTANT_RERANKER_TIMEOUT,
                "reranker returned invalid rank entry",
            ));
        }
        scores[rank.index] = rank.score;
    }
    if scores.iter().any(|score| !score.is_finite()) {
        return Err(sextant_error(
            CALYX_SEXTANT_RERANKER_TIMEOUT,
            "reranker returned incomplete rank vector",
        ));
    }
    Ok(RerankResponse {
        scores,
        zeroizing_ok: true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_real_reranker_scores_from_http_body() {
        let response = "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\r\n{\"scores\":[0.25,0.75],\"zeroizing_ok\":true}";
        let parsed = parse_http_rerank_response(response, 2).unwrap();

        assert_eq!(parsed.scores, vec![0.25, 0.75]);
        assert!(parsed.zeroizing_ok);
    }

    #[test]
    fn parses_tei_rank_array_into_candidate_order() {
        let response = "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\r\n[{\"index\":1,\"score\":0.25},{\"index\":0,\"score\":0.75}]";
        let parsed = parse_http_rerank_response(response, 2).unwrap();

        assert_eq!(parsed.scores, vec![0.75, 0.25]);
        assert!(parsed.zeroizing_ok);
    }

    #[test]
    fn rejects_mismatched_reranker_scores() {
        let response =
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\r\n{\"scores\":[0.25]}";
        let err = parse_http_rerank_response(response, 2).unwrap_err();

        assert_eq!(err.code, CALYX_SEXTANT_RERANKER_TIMEOUT);
    }

    #[test]
    fn rejects_non_success_http_status() {
        let response = "HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\n\r\n";
        let err = ensure_success_status(response).unwrap_err();

        assert_eq!(err.code, CALYX_SEXTANT_RERANKER_TIMEOUT);
        assert!(err.message.contains("503"));
    }

    #[test]
    fn read_timeout_fires_when_real_server_stalls() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("local addr");
        let server = std::thread::spawn(move || {
            // Accept the connection, read the request, then stall without
            // ever replying so the client's read timeout must fire.
            let (mut stream, _) = listener.accept().expect("accept");
            let mut buf = [0_u8; 1024];
            let _ = stream.read(&mut buf);
            std::thread::sleep(Duration::from_millis(1500));
        });

        let client = RerankerClient::new(format!("http://{addr}"), Duration::from_millis(200));
        let request = RerankRequest::new("query", vec!["candidate".to_string()]);
        let started = std::time::Instant::now();
        let err = client.rerank(&request).unwrap_err();

        assert_eq!(err.code, CALYX_SEXTANT_RERANKER_TIMEOUT);
        assert!(err.message.contains("read timeout"), "{}", err.message);
        assert!(
            started.elapsed() < Duration::from_millis(1400),
            "read returned only after the server thread exited, so the \
             configured timeout did not fire"
        );
        server.join().expect("server thread");
    }
}
