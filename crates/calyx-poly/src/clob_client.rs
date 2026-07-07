//! Read-only CLOB market-data client (issue #25).
//!
//! This module only reads public market-data surfaces. It never signs, places, cancels, or sizes
//! orders; every malformed or ambiguous market-data shape fails closed.

use std::collections::BTreeSet;
use std::time::Duration;

use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::error::{PolyError, Result};

pub use crate::clob_parse::{
    parse_clob_batch_history_value, parse_clob_books_value, parse_clob_history_value,
    parse_clob_last_trades_value, parse_clob_order_book, parse_clob_price_map_value,
    parse_clob_scalar_map_value, parse_clob_scalar_value,
};
pub use crate::clob_types::{
    CLOB_BASE_URL, ClobBatchBooksPage, ClobBatchHistoryPage, ClobBatchPricesPage,
    ClobBatchScalarsPage, ClobBookPage, ClobBookStatus, ClobClientConfig, ClobHistoryPage,
    ClobHistoryPoint, ClobJsonPage, ClobLastTrade, ClobLastTradesPage, ClobOrderBook,
    ClobPriceBatchRequest, ClobPriceHistory, ClobScalarKind, ClobScalarPage, ClobScalarQuote,
    ClobSide, ClobTokenPrices,
};

pub const ERR_CLOB_REQUEST_INVALID: &str = "CALYX_POLY_CLOB_REQUEST_INVALID";
pub const ERR_CLOB_HTTP: &str = "CALYX_POLY_CLOB_HTTP";
pub const ERR_CLOB_BODY_READ: &str = "CALYX_POLY_CLOB_BODY_READ";
pub const ERR_CLOB_JSON: &str = "CALYX_POLY_CLOB_JSON";
pub const ERR_CLOB_BOOK_INVALID: &str = "CALYX_POLY_CLOB_BOOK_INVALID";
pub const ERR_CLOB_BOOK_CROSSED: &str = "CALYX_POLY_CLOB_BOOK_CROSSED";
pub const ERR_CLOB_SCALAR_INVALID: &str = "CALYX_POLY_CLOB_SCALAR_INVALID";

pub struct ClobClient {
    config: ClobClientConfig,
    agent: ureq::Agent,
}

impl ClobClient {
    pub fn new(config: ClobClientConfig) -> Result<Self> {
        validate_config(&config)?;
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(config.timeout_secs)))
            .http_status_as_error(false)
            .build()
            .into();
        Ok(Self { config, agent })
    }

    pub fn fetch_book(&self, token_id: &str) -> Result<ClobBookPage> {
        validate_token(token_id)?;
        let page = self.get_json(format!("book?token_id={}", token_id.trim()))?;
        Ok(ClobBookPage {
            book: parse_clob_order_book(&page.value)?,
            http: page,
        })
    }

    pub fn fetch_price(&self, token_id: &str, side: ClobSide) -> Result<ClobScalarPage> {
        let kind = match side {
            ClobSide::Buy => ClobScalarKind::BuyPrice,
            ClobSide::Sell => ClobScalarKind::SellPrice,
        };
        let page = self.get_scalar(
            token_id,
            &format!("price?token_id={}&side={}", token_id.trim(), side.as_str()),
        )?;
        Ok(ClobScalarPage {
            quote: parse_clob_scalar_value(token_id, kind, "price", &page.value)?,
            http: page,
        })
    }

    pub fn fetch_midpoint(&self, token_id: &str) -> Result<ClobScalarPage> {
        self.fetch_named_scalar(token_id, "midpoint", ClobScalarKind::Midpoint, "mid")
    }

    pub fn fetch_spread(&self, token_id: &str) -> Result<ClobScalarPage> {
        self.fetch_named_scalar(token_id, "spread", ClobScalarKind::Spread, "spread")
    }

    pub fn fetch_tick_size(&self, token_id: &str) -> Result<ClobScalarPage> {
        self.fetch_named_scalar(
            token_id,
            "tick-size",
            ClobScalarKind::TickSize,
            "minimum_tick_size",
        )
    }

    pub fn fetch_prices_history(
        &self,
        token_id: &str,
        interval: &str,
        fidelity: u64,
    ) -> Result<ClobHistoryPage> {
        validate_history_request(token_id, interval, fidelity)?;
        let page = self.get_json(format!(
            "prices-history?market={}&interval={}&fidelity={fidelity}",
            token_id.trim(),
            interval.trim()
        ))?;
        Ok(ClobHistoryPage {
            history: parse_clob_history_value(token_id, &page.value)?,
            http: page,
        })
    }

    pub fn post_books(&self, token_ids: &[String]) -> Result<ClobBatchBooksPage> {
        validate_tokens(token_ids)?;
        let page = self.post_json("books", &token_array_body(token_ids))?;
        Ok(ClobBatchBooksPage {
            books: parse_clob_books_value(&page.value)?,
            http: page,
        })
    }

    pub fn post_prices(&self, requests: &[ClobPriceBatchRequest]) -> Result<ClobBatchPricesPage> {
        validate_price_requests(requests)?;
        let page = self.post_json(
            "prices",
            &serde_json::to_value(requests).map_err(|err| {
                clob_error(
                    ERR_CLOB_REQUEST_INVALID,
                    format!("encode batch price requests: {err}"),
                )
            })?,
        )?;
        Ok(ClobBatchPricesPage {
            prices: parse_clob_price_map_value(&page.value)?,
            http: page,
        })
    }

    pub fn post_midpoints(&self, token_ids: &[String]) -> Result<ClobBatchScalarsPage> {
        self.post_scalar_map("midpoints", token_ids, ClobScalarKind::Midpoint)
    }

    pub fn post_spreads(&self, token_ids: &[String]) -> Result<ClobBatchScalarsPage> {
        self.post_scalar_map("spreads", token_ids, ClobScalarKind::Spread)
    }

    pub fn post_last_trades(&self, token_ids: &[String]) -> Result<ClobLastTradesPage> {
        validate_tokens(token_ids)?;
        let page = self.post_json("last-trades-prices", &token_array_body(token_ids))?;
        Ok(ClobLastTradesPage {
            trades: parse_clob_last_trades_value(&page.value)?,
            http: page,
        })
    }

    pub fn post_batch_prices_history(
        &self,
        token_ids: &[String],
        interval: &str,
        fidelity: u64,
    ) -> Result<ClobBatchHistoryPage> {
        validate_batch_history_request(token_ids, interval, fidelity)?;
        let page = self.post_json(
            "batch-prices-history",
            &json!({"markets": token_ids, "interval": interval.trim(), "fidelity": fidelity}),
        )?;
        Ok(ClobBatchHistoryPage {
            histories: parse_clob_batch_history_value(&page.value)?,
            http: page,
        })
    }

    fn fetch_named_scalar(
        &self,
        token_id: &str,
        endpoint: &str,
        kind: ClobScalarKind,
        field: &str,
    ) -> Result<ClobScalarPage> {
        let page = self.get_scalar(
            token_id,
            &format!("{endpoint}?token_id={}", token_id.trim()),
        )?;
        Ok(ClobScalarPage {
            quote: parse_clob_scalar_value(token_id, kind, field, &page.value)?,
            http: page,
        })
    }

    fn post_scalar_map(
        &self,
        endpoint: &str,
        token_ids: &[String],
        kind: ClobScalarKind,
    ) -> Result<ClobBatchScalarsPage> {
        validate_tokens(token_ids)?;
        let page = self.post_json(endpoint, &token_array_body(token_ids))?;
        Ok(ClobBatchScalarsPage {
            quotes: parse_clob_scalar_map_value(kind, &page.value)?,
            kind,
            http: page,
        })
    }

    fn get_scalar(&self, token_id: &str, endpoint: &str) -> Result<ClobJsonPage> {
        validate_token(token_id)?;
        self.get_json(endpoint.to_string())
    }

    fn get_json(&self, endpoint: String) -> Result<ClobJsonPage> {
        let url = self.url(&endpoint);
        let mut response = self
            .agent
            .get(&url)
            .header("Accept", "application/json")
            .call()
            .map_err(|err| clob_error(ERR_CLOB_HTTP, format!("GET {url}: {err}")))?;
        self.read_json_response("GET", url, &mut response)
    }

    fn post_json(&self, endpoint: &str, body: &Value) -> Result<ClobJsonPage> {
        let url = self.url(endpoint);
        let bytes = serde_json::to_vec(body).map_err(|err| {
            clob_error(
                ERR_CLOB_REQUEST_INVALID,
                format!("encode POST {url} body: {err}"),
            )
        })?;
        let mut response = self
            .agent
            .post(&url)
            .header("Accept", "application/json")
            .header("Content-Type", "application/json")
            .send(bytes.as_slice())
            .map_err(|err| clob_error(ERR_CLOB_HTTP, format!("POST {url}: {err}")))?;
        self.read_json_response("POST", url, &mut response)
    }

    fn read_json_response(
        &self,
        method: &str,
        url: String,
        response: &mut ureq::http::Response<ureq::Body>,
    ) -> Result<ClobJsonPage> {
        let status_code = response.status().as_u16();
        let max = u64::try_from(self.config.max_body_bytes).map_err(|err| {
            clob_error(
                ERR_CLOB_REQUEST_INVALID,
                format!("convert max body bytes: {err}"),
            )
        })?;
        let bytes = response
            .body_mut()
            .with_config()
            .limit(max)
            .read_to_vec()
            .map_err(|err| clob_error(ERR_CLOB_BODY_READ, format!("read {method} {url}: {err}")))?;
        if !(200..300).contains(&status_code) {
            return Err(clob_error(
                ERR_CLOB_HTTP,
                format!("{method} {url} returned HTTP {status_code}"),
            ));
        }
        let value: Value = serde_json::from_slice(&bytes)
            .map_err(|err| clob_error(ERR_CLOB_JSON, format!("decode {method} {url}: {err}")))?;
        Ok(ClobJsonPage {
            method: method.to_string(),
            url,
            status_code,
            body_bytes: bytes.len() as u64,
            body_sha256: sha256_hex(&bytes),
            raw_body: bytes,
            value,
        })
    }

    fn url(&self, endpoint: &str) -> String {
        format!(
            "{}/{}",
            self.config.base_url.trim_end_matches('/'),
            endpoint.trim_start_matches('/')
        )
    }
}

fn validate_config(config: &ClobClientConfig) -> Result<()> {
    if config.base_url.trim().is_empty() || config.timeout_secs == 0 || config.max_body_bytes == 0 {
        return Err(clob_error(
            ERR_CLOB_REQUEST_INVALID,
            "CLOB base_url, timeout_secs, and max_body_bytes must be non-empty",
        ));
    }
    Ok(())
}

fn validate_history_request(token_id: &str, interval: &str, fidelity: u64) -> Result<()> {
    validate_token(token_id)?;
    if interval.trim().is_empty() || has_url_separator(interval) || fidelity == 0 {
        return Err(clob_error(
            ERR_CLOB_REQUEST_INVALID,
            "history interval must be URL-safe and fidelity must be positive",
        ));
    }
    Ok(())
}

fn validate_batch_history_request(
    token_ids: &[String],
    interval: &str,
    fidelity: u64,
) -> Result<()> {
    validate_tokens(token_ids)?;
    validate_history_request(&token_ids[0], interval, fidelity)?;
    let unique = token_ids
        .iter()
        .map(|token| token.trim())
        .collect::<BTreeSet<_>>();
    if unique.len() > 20 {
        return Err(clob_error(
            ERR_CLOB_REQUEST_INVALID,
            format!(
                "batch prices-history unique market count {} exceeds 20",
                unique.len()
            ),
        ));
    }
    Ok(())
}

fn validate_price_requests(requests: &[ClobPriceBatchRequest]) -> Result<()> {
    if requests.is_empty() {
        return Err(clob_error(
            ERR_CLOB_REQUEST_INVALID,
            "batch request is empty",
        ));
    }
    for request in requests {
        validate_token(&request.token_id)?;
        if request
            .side
            .as_deref()
            .is_some_and(|side| side.trim().is_empty() || has_url_separator(side))
        {
            return Err(clob_error(
                ERR_CLOB_REQUEST_INVALID,
                "batch price side is malformed",
            ));
        }
    }
    Ok(())
}

fn validate_tokens(token_ids: &[String]) -> Result<()> {
    if token_ids.is_empty() {
        return Err(clob_error(ERR_CLOB_REQUEST_INVALID, "token list is empty"));
    }
    for token_id in token_ids {
        validate_token(token_id)?;
    }
    Ok(())
}

fn validate_token(token_id: &str) -> Result<()> {
    if token_id.trim().is_empty() || has_url_separator(token_id) {
        return Err(clob_error(
            ERR_CLOB_REQUEST_INVALID,
            "CLOB token id must be non-empty and URL-safe",
        ));
    }
    Ok(())
}

fn has_url_separator(value: &str) -> bool {
    value
        .chars()
        .any(|ch| ch.is_whitespace() || matches!(ch, '&' | '?' | '#'))
}

fn token_array_body(token_ids: &[String]) -> Value {
    Value::Array(
        token_ids
            .iter()
            .map(|token_id| json!({"token_id": token_id}))
            .collect(),
    )
}

pub(crate) fn clob_error(code: impl Into<String>, message: impl Into<String>) -> PolyError {
    PolyError::raw_source(code, message)
}

pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}
