//! Gamma metadata endpoint parsing for events, series, and tags (issue #24).

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{PolyError, Result};
use crate::gamma_client::{ERR_GAMMA_REQUEST_INVALID, GAMMA_CRYPTO_TAG_ID, GammaClient};

pub const ERR_GAMMA_METADATA_INVALID: &str = "CALYX_POLY_GAMMA_METADATA_INVALID";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GammaEventsRequest {
    pub active: Option<bool>,
    pub closed: Option<bool>,
    pub tag_id: Option<u64>,
    pub limit: usize,
}

impl GammaEventsRequest {
    pub fn crypto_active(limit: usize) -> Self {
        Self {
            active: Some(true),
            closed: Some(false),
            tag_id: Some(GAMMA_CRYPTO_TAG_ID),
            limit,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GammaSeriesRequest {
    pub active: Option<bool>,
    pub closed: Option<bool>,
    pub tag_id: Option<u64>,
    pub limit: usize,
}

impl GammaSeriesRequest {
    pub fn with_limit(limit: usize) -> Self {
        Self {
            active: None,
            closed: None,
            tag_id: None,
            limit,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GammaTagsRequest {
    pub limit: usize,
}

impl GammaTagsRequest {
    pub fn with_limit(limit: usize) -> Self {
        Self { limit }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GammaEventsPage {
    pub url: String,
    pub status_code: u16,
    pub body_bytes: u64,
    pub body_sha256: String,
    #[serde(skip, default)]
    pub raw_body: Vec<u8>,
    pub events: Vec<GammaEventRecord>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GammaSeriesPage {
    pub url: String,
    pub status_code: u16,
    pub body_bytes: u64,
    pub body_sha256: String,
    #[serde(skip, default)]
    pub raw_body: Vec<u8>,
    pub series: Vec<GammaSeriesRecord>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GammaTagsPage {
    pub url: String,
    pub status_code: u16,
    pub body_bytes: u64,
    pub body_sha256: String,
    #[serde(skip, default)]
    pub raw_body: Vec<u8>,
    pub tags: Vec<GammaTagRecord>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GammaEventRecord {
    pub event_id: String,
    pub slug: Option<String>,
    pub title: Option<String>,
    pub active: bool,
    pub closed: bool,
    pub market_ids: Vec<String>,
    pub condition_ids: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GammaSeriesRecord {
    pub series_id: String,
    pub ticker: Option<String>,
    pub slug: Option<String>,
    pub title: Option<String>,
    pub active: bool,
    pub closed: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GammaTagRecord {
    pub tag_id: String,
    pub label: String,
    pub slug: String,
}

impl GammaClient {
    pub fn fetch_events(&self, request: &GammaEventsRequest) -> Result<GammaEventsPage> {
        validate_limit(request.limit)?;
        let page = self.fetch_json(events_url(request))?;
        Ok(GammaEventsPage {
            url: page.url,
            status_code: page.status_code,
            body_bytes: page.body_bytes,
            body_sha256: page.body_sha256,
            raw_body: page.raw_body,
            events: parse_gamma_events_value(&page.value)?,
        })
    }

    pub fn fetch_series(&self, request: &GammaSeriesRequest) -> Result<GammaSeriesPage> {
        validate_limit(request.limit)?;
        let page = self.fetch_json(series_url(request))?;
        Ok(GammaSeriesPage {
            url: page.url,
            status_code: page.status_code,
            body_bytes: page.body_bytes,
            body_sha256: page.body_sha256,
            raw_body: page.raw_body,
            series: parse_gamma_series_value(&page.value)?,
        })
    }

    pub fn fetch_tags(&self, request: &GammaTagsRequest) -> Result<GammaTagsPage> {
        validate_limit(request.limit)?;
        let page = self.fetch_json(tags_url(request))?;
        Ok(GammaTagsPage {
            url: page.url,
            status_code: page.status_code,
            body_bytes: page.body_bytes,
            body_sha256: page.body_sha256,
            raw_body: page.raw_body,
            tags: parse_gamma_tags_value(&page.value)?,
        })
    }
}

pub fn parse_gamma_events_value(value: &Value) -> Result<Vec<GammaEventRecord>> {
    rows(value)?.iter().map(parse_event).collect()
}

pub fn parse_gamma_series_value(value: &Value) -> Result<Vec<GammaSeriesRecord>> {
    rows(value)?.iter().map(parse_series).collect()
}

pub fn parse_gamma_tags_value(value: &Value) -> Result<Vec<GammaTagRecord>> {
    rows(value)?.iter().map(parse_tag).collect()
}

fn parse_event(value: &Value) -> Result<GammaEventRecord> {
    Ok(GammaEventRecord {
        event_id: required_string(value, "id")?,
        slug: optional_string(value, "slug")?,
        title: optional_string(value, "title")?,
        active: required_bool(value, "active")?,
        closed: required_bool(value, "closed")?,
        market_ids: nested_market_strings(value, "id")?,
        condition_ids: nested_market_strings(value, "conditionId")?,
    })
}

fn parse_series(value: &Value) -> Result<GammaSeriesRecord> {
    Ok(GammaSeriesRecord {
        series_id: required_string(value, "id")?,
        ticker: optional_string(value, "ticker")?,
        slug: optional_string(value, "slug")?,
        title: optional_string(value, "title")?,
        active: required_bool(value, "active")?,
        closed: required_bool(value, "closed")?,
    })
}

fn parse_tag(value: &Value) -> Result<GammaTagRecord> {
    Ok(GammaTagRecord {
        tag_id: required_string(value, "id")?,
        label: required_string(value, "label")?,
        slug: required_string(value, "slug")?,
    })
}

fn rows(value: &Value) -> Result<Vec<Value>> {
    match value {
        Value::Array(rows) => Ok(rows.clone()),
        Value::Object(map) => Ok(map
            .get("data")
            .or_else(|| map.get("events"))
            .or_else(|| map.get("series"))
            .or_else(|| map.get("tags"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()),
        _ => Err(metadata_error(
            ERR_GAMMA_METADATA_INVALID,
            "Gamma metadata response must be an array or object containing rows",
        )),
    }
}

fn nested_market_strings(value: &Value, field: &str) -> Result<Vec<String>> {
    let Some(markets) = value.get("markets").and_then(Value::as_array) else {
        return Ok(Vec::new());
    };
    markets
        .iter()
        .filter_map(|market| market.get(field))
        .map(|raw| string_value(raw, field))
        .collect()
}

fn events_url(request: &GammaEventsRequest) -> String {
    let mut parts = vec![format!("limit={}", request.limit)];
    push_bool(&mut parts, "active", request.active);
    push_bool(&mut parts, "closed", request.closed);
    if let Some(tag_id) = request.tag_id {
        parts.push(format!("tag_id={tag_id}"));
    }
    format!(
        "https://gamma-api.polymarket.com/events?{}",
        parts.join("&")
    )
}

fn series_url(request: &GammaSeriesRequest) -> String {
    let mut parts = vec![format!("limit={}", request.limit)];
    push_bool(&mut parts, "active", request.active);
    push_bool(&mut parts, "closed", request.closed);
    if let Some(tag_id) = request.tag_id {
        parts.push(format!("tag_id={tag_id}"));
    }
    format!(
        "https://gamma-api.polymarket.com/series?{}",
        parts.join("&")
    )
}

fn tags_url(request: &GammaTagsRequest) -> String {
    format!(
        "https://gamma-api.polymarket.com/tags?limit={}",
        request.limit
    )
}

fn push_bool(parts: &mut Vec<String>, name: &str, value: Option<bool>) {
    if let Some(value) = value {
        parts.push(format!("{name}={value}"));
    }
}

fn validate_limit(limit: usize) -> Result<()> {
    if limit > 500 {
        return Err(metadata_error(
            ERR_GAMMA_REQUEST_INVALID,
            format!("Gamma metadata limit {limit} exceeds 500"),
        ));
    }
    Ok(())
}

fn required_string(value: &Value, field: &str) -> Result<String> {
    optional_string(value, field)?
        .filter(|text| !text.trim().is_empty())
        .ok_or_else(|| {
            metadata_error(
                ERR_GAMMA_METADATA_INVALID,
                format!("Gamma metadata row missing required field {field}"),
            )
        })
}

fn optional_string(value: &Value, field: &str) -> Result<Option<String>> {
    match value.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(raw) => Ok(Some(string_value(raw, field)?)),
    }
}

fn string_value(value: &Value, field: &str) -> Result<String> {
    match value {
        Value::String(text) => Ok(text.clone()),
        Value::Number(number) => Ok(number.to_string()),
        other => Err(metadata_error(
            ERR_GAMMA_METADATA_INVALID,
            format!("Gamma metadata field {field} expected string-compatible value, got {other}"),
        )),
    }
}

fn required_bool(value: &Value, field: &str) -> Result<bool> {
    match value.get(field) {
        Some(Value::Bool(value)) => Ok(*value),
        _ => Err(metadata_error(
            ERR_GAMMA_METADATA_INVALID,
            format!("Gamma metadata row missing required bool field {field}"),
        )),
    }
}

fn metadata_error(code: impl Into<String>, message: impl Into<String>) -> PolyError {
    PolyError::raw_source(code, message)
}
