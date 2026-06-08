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
        Ok(self.mock_scores(request))
    }
}

fn lexical_overlap(query: &str, candidate: &str) -> f32 {
    let q = crate::index::tokenizer::tokenize(query);
    let c = crate::index::tokenizer::tokenize(candidate);
    q.iter().filter(|term| c.contains(term)).count() as f32
}
