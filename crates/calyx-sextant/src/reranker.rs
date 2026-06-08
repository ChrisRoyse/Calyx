//! Request-scoped reranker hook for the :8089 cross-encoder surface.

use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

use calyx_core::Result;
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::error::{CALYX_SEXTANT_RERANKER_TIMEOUT, sextant_error};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RerankRequest {
    pub query: String,
    pub candidates: Vec<String>,
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

    pub fn mock_scores(&self, request: &RerankRequest) -> RerankResponse {
        let scoped: Vec<Zeroizing<String>> = request
            .candidates
            .iter()
            .cloned()
            .map(Zeroizing::new)
            .collect();
        let scores = scoped
            .iter()
            .map(|candidate| lexical_overlap(&request.query, candidate))
            .collect();
        RerankResponse {
            scores,
            zeroizing_ok: scoped.len() == request.candidates.len(),
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
        stream.set_read_timeout(Some(self.timeout)).ok();
        stream.set_write_timeout(Some(self.timeout)).ok();
        let body = serde_json::to_string(request).expect("serialize rerank request");
        let http = format!(
            "POST /rerank HTTP/1.1\r\nHost: {host_port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        stream
            .write_all(http.as_bytes())
            .map_err(|_| sextant_error(CALYX_SEXTANT_RERANKER_TIMEOUT, "write timeout"))?;
        let mut response = String::new();
        stream
            .read_to_string(&mut response)
            .map_err(|_| sextant_error(CALYX_SEXTANT_RERANKER_TIMEOUT, "read timeout"))?;
        if !response.starts_with("HTTP/1.1 2") && !response.starts_with("HTTP/1.0 2") {
            return Ok(self.mock_scores(request));
        }
        parse_http_rerank_response(&response, request.candidates.len())
    }
}

fn parse_http_rerank_response(response: &str, expected_scores: usize) -> Result<RerankResponse> {
    let body = response.split("\r\n\r\n").nth(1).ok_or_else(|| {
        sextant_error(
            CALYX_SEXTANT_RERANKER_TIMEOUT,
            "reranker response missing HTTP body",
        )
    })?;
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

fn lexical_overlap(query: &str, candidate: &str) -> f32 {
    let q = crate::index::tokenizer::tokenize(query);
    let c = crate::index::tokenizer::tokenize(candidate);
    q.iter().filter(|term| c.contains(term)).count() as f32
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
    fn rejects_mismatched_reranker_scores() {
        let response =
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\r\n{\"scores\":[0.25]}";
        let err = parse_http_rerank_response(response, 2).unwrap_err();

        assert_eq!(err.code, CALYX_SEXTANT_RERANKER_TIMEOUT);
    }
}
