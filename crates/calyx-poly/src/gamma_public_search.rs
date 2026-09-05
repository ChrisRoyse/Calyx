//! Public Gamma search discovery for near-term crypto markets.
//!
//! The normal tagged `/markets?tag_id=21` page can miss recurring same-day price ladders. The
//! public-search endpoint returns events with nested markets, so this module flattens those market
//! rows and reuses the strict Gamma market parser.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{PolyError, Result};
use crate::gamma_client::{
    ERR_GAMMA_JSON, ERR_GAMMA_REQUEST_INVALID, GammaClient, GammaMarketRecord, parse_gamma_market,
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GammaPublicSearchRequest {
    pub query: String,
    pub limit_per_type: usize,
}

impl GammaPublicSearchRequest {
    pub fn new(query: impl Into<String>, limit_per_type: usize) -> Self {
        Self {
            query: query.into(),
            limit_per_type,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GammaPublicSearchPage {
    pub url: String,
    pub status_code: u16,
    pub body_bytes: u64,
    pub body_sha256: String,
    #[serde(skip, default)]
    pub raw_body: Vec<u8>,
    pub markets: Vec<GammaMarketRecord>,
}

impl GammaClient {
    pub fn fetch_public_search_markets(
        &self,
        request: &GammaPublicSearchRequest,
    ) -> Result<GammaPublicSearchPage> {
        validate_request(request)?;
        let url = public_search_url(request);
        let page = self.fetch_json(url)?;
        Ok(GammaPublicSearchPage {
            url: page.url,
            status_code: page.status_code,
            body_bytes: page.body_bytes,
            body_sha256: page.body_sha256,
            raw_body: page.raw_body,
            markets: parse_gamma_public_search_markets_value(&page.value)?,
        })
    }
}

pub fn parse_gamma_public_search_markets_value(value: &Value) -> Result<Vec<GammaMarketRecord>> {
    let Some(map) = value.as_object() else {
        return Err(gamma_public_search_error(
            ERR_GAMMA_JSON,
            "Gamma public-search response must be an object",
        ));
    };
    let mut rows = Vec::new();
    if let Some(markets) = map.get("markets").and_then(Value::as_array) {
        rows.extend(markets.iter());
    }
    if let Some(events) = map.get("events").and_then(Value::as_array) {
        for event in events {
            if let Some(markets) = event.get("markets").and_then(Value::as_array) {
                rows.extend(markets.iter());
            }
        }
    }
    rows.into_iter().map(parse_gamma_market).collect()
}

fn validate_request(request: &GammaPublicSearchRequest) -> Result<()> {
    let query = request.query.trim();
    if query.is_empty() || query.len() > 80 || request.limit_per_type == 0 {
        return Err(gamma_public_search_error(
            ERR_GAMMA_REQUEST_INVALID,
            "Gamma public-search query must be 1..=80 chars and limit_per_type must be positive",
        ));
    }
    if request.limit_per_type > 20 {
        return Err(gamma_public_search_error(
            ERR_GAMMA_REQUEST_INVALID,
            format!(
                "Gamma public-search limit_per_type {} exceeds 20",
                request.limit_per_type
            ),
        ));
    }
    Ok(())
}

fn public_search_url(request: &GammaPublicSearchRequest) -> String {
    format!(
        "https://gamma-api.polymarket.com/public-search?q={}&limit_per_type={}",
        encode_query(request.query.trim()),
        request.limit_per_type
    )
}

fn encode_query(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char);
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

fn gamma_public_search_error(code: impl Into<String>, message: impl Into<String>) -> PolyError {
    PolyError::raw_source(code, message)
}
