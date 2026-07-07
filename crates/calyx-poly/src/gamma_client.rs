//! Read-only Gamma market discovery client (issue #24).
//!
//! Gamma is the entry point for Polymarket market identity: market id, condition id, outcome token
//! ids, event id, outcome labels/prices, and metadata that the CLOB/Data clients join against.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::error::{PolyError, Result};
use crate::gamma_time::first_timestamp;

pub const GAMMA_BASE_URL: &str = "https://gamma-api.polymarket.com";
pub const GAMMA_CRYPTO_TAG_ID: u64 = 21;
pub const ERR_GAMMA_REQUEST_INVALID: &str = "CALYX_POLY_GAMMA_REQUEST_INVALID";
pub const ERR_GAMMA_HTTP: &str = "CALYX_POLY_GAMMA_HTTP";
pub const ERR_GAMMA_BODY_READ: &str = "CALYX_POLY_GAMMA_BODY_READ";
pub const ERR_GAMMA_JSON: &str = "CALYX_POLY_GAMMA_JSON";
pub const ERR_GAMMA_MARKET_INVALID: &str = "CALYX_POLY_GAMMA_MARKET_INVALID";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GammaClientConfig {
    pub base_url: String,
    pub timeout_secs: u64,
    pub max_body_bytes: usize,
}

impl Default for GammaClientConfig {
    fn default() -> Self {
        Self {
            base_url: GAMMA_BASE_URL.to_string(),
            timeout_secs: 20,
            max_body_bytes: 5 * 1024 * 1024,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GammaMarketsRequest {
    pub active: Option<bool>,
    pub closed: Option<bool>,
    pub tag_id: Option<u64>,
    pub limit: usize,
}

impl GammaMarketsRequest {
    pub fn crypto_active(limit: usize) -> Self {
        Self {
            active: Some(true),
            closed: Some(false),
            tag_id: Some(GAMMA_CRYPTO_TAG_ID),
            limit,
        }
    }

    pub fn crypto_closed(limit: usize) -> Self {
        Self {
            active: None,
            closed: Some(true),
            tag_id: Some(GAMMA_CRYPTO_TAG_ID),
            limit,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GammaMarketsPage {
    pub url: String,
    pub status_code: u16,
    pub body_bytes: u64,
    pub body_sha256: String,
    #[serde(skip, default)]
    pub raw_body: Vec<u8>,
    pub markets: Vec<GammaMarketRecord>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GammaOutcomeShape {
    Binary,
    NonBinary,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GammaMarketRecord {
    pub market_id: String,
    pub condition_id: String,
    pub slug: Option<String>,
    pub question: Option<String>,
    pub event_id: Option<String>,
    pub event_slug: Option<String>,
    pub active: bool,
    pub closed: bool,
    pub neg_risk: bool,
    pub enable_order_book: Option<bool>,
    pub outcomes: Vec<String>,
    pub outcome_prices: Vec<f64>,
    pub clob_token_ids: Vec<String>,
    pub outcome_shape: GammaOutcomeShape,
    pub category: Option<String>,
    pub resolution_source: Option<String>,
    pub volume_24h: Option<f64>,
    pub liquidity: Option<f64>,
    pub best_bid: Option<f64>,
    pub best_ask: Option<f64>,
    pub spread: Option<f64>,
    pub last_trade_price: Option<f64>,
    pub end_ts: Option<u64>,
    pub join_key: GammaJoinKey,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GammaJoinKey {
    pub market_id: String,
    pub condition_id: String,
    pub token_ids: Vec<String>,
    pub event_id: Option<String>,
}

pub struct GammaClient {
    config: GammaClientConfig,
    agent: ureq::Agent,
}

pub(crate) struct GammaJsonPage {
    pub(crate) url: String,
    pub(crate) status_code: u16,
    pub(crate) body_bytes: u64,
    pub(crate) body_sha256: String,
    pub(crate) raw_body: Vec<u8>,
    pub(crate) value: Value,
}

impl GammaClient {
    pub fn new(config: GammaClientConfig) -> Result<Self> {
        validate_config(&config)?;
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(config.timeout_secs)))
            .http_status_as_error(false)
            .build()
            .into();
        Ok(Self { config, agent })
    }

    pub fn fetch_markets(&self, request: &GammaMarketsRequest) -> Result<GammaMarketsPage> {
        validate_request(request)?;
        let url = market_url(&self.config.base_url, request);
        let page = self.fetch_json(url)?;
        Ok(GammaMarketsPage {
            url: page.url,
            status_code: page.status_code,
            body_bytes: page.body_bytes,
            body_sha256: page.body_sha256,
            raw_body: page.raw_body,
            markets: parse_gamma_markets_value(&page.value)?,
        })
    }

    pub(crate) fn fetch_json(&self, url: String) -> Result<GammaJsonPage> {
        let mut response = self
            .agent
            .get(&url)
            .header("Accept", "application/json")
            .call()
            .map_err(|err| gamma_error(ERR_GAMMA_HTTP, format!("GET {url}: {err}")))?;
        let status_code = response.status().as_u16();
        let max = u64::try_from(self.config.max_body_bytes).map_err(|err| {
            gamma_error(
                ERR_GAMMA_REQUEST_INVALID,
                format!(
                    "convert max_body_bytes {}: {err}",
                    self.config.max_body_bytes
                ),
            )
        })?;
        let bytes = response
            .body_mut()
            .with_config()
            .limit(max)
            .read_to_vec()
            .map_err(|err| gamma_error(ERR_GAMMA_BODY_READ, format!("read {url}: {err}")))?;
        if !(200..300).contains(&status_code) {
            return Err(gamma_error(
                ERR_GAMMA_HTTP,
                format!("GET {url} returned HTTP {status_code}"),
            ));
        }
        let value: Value = serde_json::from_slice(&bytes)
            .map_err(|err| gamma_error(ERR_GAMMA_JSON, format!("decode {url}: {err}")))?;
        Ok(GammaJsonPage {
            url,
            status_code,
            body_bytes: bytes.len() as u64,
            body_sha256: sha256_hex(&bytes),
            raw_body: bytes,
            value,
        })
    }
}

pub fn parse_gamma_markets_value(value: &Value) -> Result<Vec<GammaMarketRecord>> {
    let rows = match value {
        Value::Array(rows) => rows.as_slice(),
        Value::Object(map) => map
            .get("data")
            .or_else(|| map.get("markets"))
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or(&[]),
        _ => {
            return Err(gamma_error(
                ERR_GAMMA_JSON,
                "Gamma markets response must be an array or an object containing data/markets",
            ));
        }
    };
    rows.iter().map(parse_gamma_market).collect()
}

pub fn parse_gamma_market(value: &Value) -> Result<GammaMarketRecord> {
    let market_id = required_string(value, "id")?;
    let condition_id = required_string(value, "conditionId")?;
    let outcomes = string_array_field(value, "outcomes")?;
    let outcome_prices = number_array_field(value, "outcomePrices")?;
    let clob_token_ids = string_array_field(value, "clobTokenIds")?;
    if outcomes.is_empty() || outcomes.len() != outcome_prices.len() {
        return Err(invalid_market(
            &market_id,
            "outcomes and outcomePrices must be non-empty and equal length",
        ));
    }
    if clob_token_ids.len() != outcomes.len() {
        return Err(invalid_market(
            &market_id,
            "clobTokenIds must have one token id per outcome",
        ));
    }
    let active = required_bool(value, "active")?;
    let closed = required_bool(value, "closed")?;
    let (event_id, event_slug) = first_event(value)?;
    let outcome_shape = if outcomes.len() == 2 {
        GammaOutcomeShape::Binary
    } else {
        GammaOutcomeShape::NonBinary
    };
    let join_key = GammaJoinKey {
        market_id: market_id.clone(),
        condition_id: condition_id.clone(),
        token_ids: clob_token_ids.clone(),
        event_id: event_id.clone(),
    };
    Ok(GammaMarketRecord {
        market_id,
        condition_id,
        slug: optional_string(value, "slug")?,
        question: optional_string(value, "question")?,
        event_id,
        event_slug,
        active,
        closed,
        neg_risk: optional_bool(value, "negRisk")?.unwrap_or(false),
        enable_order_book: optional_bool(value, "enableOrderBook")?,
        outcomes,
        outcome_prices,
        clob_token_ids,
        outcome_shape,
        category: optional_string(value, "category")?,
        resolution_source: optional_string(value, "resolutionSource")?,
        volume_24h: first_number(value, &["volume24hrClob", "volume24hr"])?,
        liquidity: first_number(value, &["liquidityClob", "liquidityNum", "liquidity"])?,
        best_bid: optional_number(value, "bestBid")?,
        best_ask: optional_number(value, "bestAsk")?,
        spread: optional_number(value, "spread")?,
        last_trade_price: optional_number(value, "lastTradePrice")?,
        end_ts: first_timestamp(value, &["endDate", "endDateIso", "end_date"])?,
        join_key,
    })
}

fn validate_config(config: &GammaClientConfig) -> Result<()> {
    if config.base_url.trim().is_empty() || config.timeout_secs == 0 || config.max_body_bytes == 0 {
        return Err(gamma_error(
            ERR_GAMMA_REQUEST_INVALID,
            "Gamma base_url, timeout_secs, and max_body_bytes must be non-empty",
        ));
    }
    Ok(())
}

fn validate_request(request: &GammaMarketsRequest) -> Result<()> {
    if request.limit > 500 {
        return Err(gamma_error(
            ERR_GAMMA_REQUEST_INVALID,
            format!("Gamma markets limit {} exceeds 500", request.limit),
        ));
    }
    Ok(())
}

fn market_url(base_url: &str, request: &GammaMarketsRequest) -> String {
    let mut parts = vec![format!("limit={}", request.limit)];
    if let Some(active) = request.active {
        parts.push(format!("active={active}"));
    }
    if let Some(closed) = request.closed {
        parts.push(format!("closed={closed}"));
    }
    if let Some(tag_id) = request.tag_id {
        parts.push(format!("tag_id={tag_id}"));
    }
    format!(
        "{}/markets?{}",
        base_url.trim_end_matches('/'),
        parts.join("&")
    )
}

fn first_event(value: &Value) -> Result<(Option<String>, Option<String>)> {
    let Some(events) = value.get("events") else {
        return Ok((None, None));
    };
    let Some(first) = events.as_array().and_then(|events| events.first()) else {
        return Ok((None, None));
    };
    Ok((
        optional_string(first, "id")?,
        optional_string(first, "slug")?,
    ))
}

fn string_array_field(value: &Value, field: &str) -> Result<Vec<String>> {
    parse_array_field(value, field)?
        .into_iter()
        .map(|item| match item {
            Value::String(text) if !text.trim().is_empty() => Ok(text),
            other => Err(gamma_error(
                ERR_GAMMA_MARKET_INVALID,
                format!("field {field} contained non-string array item {other}"),
            )),
        })
        .collect()
}

fn number_array_field(value: &Value, field: &str) -> Result<Vec<f64>> {
    parse_array_field(value, field)?
        .iter()
        .map(|item| number_value(item, field))
        .collect()
}

fn parse_array_field(value: &Value, field: &str) -> Result<Vec<Value>> {
    let raw = value.get(field).ok_or_else(|| {
        gamma_error(
            ERR_GAMMA_MARKET_INVALID,
            format!("Gamma market missing required field {field}"),
        )
    })?;
    match raw {
        Value::Array(items) => Ok(items.clone()),
        Value::String(text) => {
            let parsed: Value = serde_json::from_str(text).map_err(|err| {
                gamma_error(
                    ERR_GAMMA_MARKET_INVALID,
                    format!("Gamma field {field} is not valid JSON array string: {err}"),
                )
            })?;
            parsed.as_array().cloned().ok_or_else(|| {
                gamma_error(
                    ERR_GAMMA_MARKET_INVALID,
                    format!("Gamma field {field} JSON string did not decode to an array"),
                )
            })
        }
        _ => Err(gamma_error(
            ERR_GAMMA_MARKET_INVALID,
            format!("Gamma field {field} must be an array or JSON-encoded array string"),
        )),
    }
}

fn required_string(value: &Value, field: &str) -> Result<String> {
    optional_string(value, field)?
        .filter(|text| !text.trim().is_empty())
        .ok_or_else(|| {
            gamma_error(
                ERR_GAMMA_MARKET_INVALID,
                format!("Gamma market missing required field {field}"),
            )
        })
}

fn optional_string(value: &Value, field: &str) -> Result<Option<String>> {
    match value.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) => Ok(Some(text.clone())),
        Some(Value::Number(number)) => Ok(Some(number.to_string())),
        Some(other) => Err(gamma_error(
            ERR_GAMMA_MARKET_INVALID,
            format!("Gamma field {field} expected string-compatible value, got {other}"),
        )),
    }
}

fn required_bool(value: &Value, field: &str) -> Result<bool> {
    optional_bool(value, field)?.ok_or_else(|| {
        gamma_error(
            ERR_GAMMA_MARKET_INVALID,
            format!("Gamma market missing required bool field {field}"),
        )
    })
}

fn optional_bool(value: &Value, field: &str) -> Result<Option<bool>> {
    match value.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(value)) => Ok(Some(*value)),
        Some(other) => Err(gamma_error(
            ERR_GAMMA_MARKET_INVALID,
            format!("Gamma field {field} expected bool, got {other}"),
        )),
    }
}

fn first_number(value: &Value, fields: &[&str]) -> Result<Option<f64>> {
    for field in fields {
        if value.get(*field).is_some() {
            return optional_number(value, field);
        }
    }
    Ok(None)
}

fn optional_number(value: &Value, field: &str) -> Result<Option<f64>> {
    match value.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(raw) => Ok(Some(number_value(raw, field)?)),
    }
}

fn number_value(value: &Value, field: &str) -> Result<f64> {
    let parsed = match value {
        Value::Number(number) => number.as_f64(),
        Value::String(text) => text.parse::<f64>().ok(),
        _ => None,
    }
    .filter(|value| value.is_finite());
    parsed.ok_or_else(|| {
        gamma_error(
            ERR_GAMMA_MARKET_INVALID,
            format!("Gamma numeric field {field} is missing, malformed, or non-finite"),
        )
    })
}

fn invalid_market(market_id: &str, message: impl Into<String>) -> PolyError {
    gamma_error(
        ERR_GAMMA_MARKET_INVALID,
        format!("Gamma market {market_id}: {}", message.into()),
    )
}

fn gamma_error(code: impl Into<String>, message: impl Into<String>) -> PolyError {
    PolyError::raw_source(code, message)
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}
