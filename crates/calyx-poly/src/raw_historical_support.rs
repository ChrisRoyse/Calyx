use std::fs;
use std::path::Path;

use serde_json::Value;

use crate::rate_limit_governor::{
    RateLimitEndpoint, RateLimitedHttpOutcome, execute_rate_limited_request, parse_retry_after_ms,
};
use crate::raw_source_support::sha256_hex;
use crate::{PolyError, Result};

const HF_TREE_MAX_PAGES: usize = 100;

pub(crate) fn execute_get(
    agent: &ureq::Agent,
    name: &str,
    url: &str,
    format: HistoricalFormat,
    max_body_bytes: u64,
) -> Result<(Option<u16>, Vec<u8>, Option<String>)> {
    if matches!(format, HistoricalFormat::HfTreeJson) {
        return execute_hf_tree_get(agent, name, url, max_body_bytes);
    }
    let endpoint = RateLimitEndpoint::new("historical-dump", name, "GET");
    execute_rate_limited_request(&endpoint, || {
        let result = agent.get(url).header("Accept", "*/*").call();
        let mut status_code = None;
        let mut retry_after_ms = None;
        let mut bytes = Vec::new();
        let mut transport_error = None;
        match result {
            Ok(mut response) => {
                status_code = Some(response.status().as_u16());
                retry_after_ms = parse_retry_after_ms(
                    response
                        .headers()
                        .get("retry-after")
                        .and_then(|value| value.to_str().ok()),
                );
                bytes = response
                    .body_mut()
                    .with_config()
                    .limit(max_body_bytes)
                    .read_to_vec()
                    .map_err(|err| {
                        PolyError::raw_source(
                            "POLY_RAW_SOURCE_HISTORICAL_BODY_READ_FAILED",
                            format!("read historical body for {name}: {err}"),
                        )
                    })?;
            }
            Err(err) => transport_error = Some(err.to_string()),
        }
        Ok(RateLimitedHttpOutcome {
            status_code,
            retry_after_ms,
            value: (status_code, bytes, transport_error),
        })
    })
}

fn execute_hf_tree_get(
    agent: &ureq::Agent,
    name: &str,
    url: &str,
    max_body_bytes: u64,
) -> Result<(Option<u16>, Vec<u8>, Option<String>)> {
    let mut next_url = Some(url.to_string());
    let mut status_code = None;
    let mut records = Vec::new();
    let mut page_count = 0usize;
    let endpoint = RateLimitEndpoint::new("historical-dump", name, "GET");
    while let Some(page_url) = next_url {
        if page_count >= HF_TREE_MAX_PAGES {
            return Err(PolyError::raw_source(
                "POLY_RAW_SOURCE_HF_TREE_PAGINATION_LIMIT",
                format!(
                    "Hugging Face tree probe {name} exceeded {HF_TREE_MAX_PAGES} pages at {page_url}"
                ),
            ));
        }
        let (page_status, page_bytes, next_from_header, transport_error) =
            execute_rate_limited_request(&endpoint, || {
                let result = agent
                    .get(&page_url)
                    .header("Accept", "application/json")
                    .call();
                let mut response = match result {
                    Ok(response) => response,
                    Err(err) => {
                        return Ok(RateLimitedHttpOutcome {
                            status_code: None,
                            retry_after_ms: None,
                            value: (None, Vec::new(), None, Some(err.to_string())),
                        });
                    }
                };
                let page_status = response.status().as_u16();
                let retry_after_ms = parse_retry_after_ms(
                    response
                        .headers()
                        .get("retry-after")
                        .and_then(|value| value.to_str().ok()),
                );
                let next_from_header = response
                    .headers()
                    .get("link")
                    .and_then(|value| value.to_str().ok())
                    .and_then(next_hf_tree_url);
                let page_bytes = response
                    .body_mut()
                    .with_config()
                    .limit(max_body_bytes)
                    .read_to_vec()
                    .map_err(|err| {
                        PolyError::raw_source(
                            "POLY_RAW_SOURCE_HF_TREE_BODY_READ_FAILED",
                            format!("read Hugging Face tree body for {name}: {err}"),
                        )
                    })?;
                Ok(RateLimitedHttpOutcome {
                    status_code: Some(page_status),
                    retry_after_ms,
                    value: (Some(page_status), page_bytes, next_from_header, None),
                })
            })?;
        if let Some(err) = transport_error {
            return Ok((status_code, Vec::new(), Some(err)));
        }
        page_count += 1;
        let Some(page_status) = page_status else {
            return Ok((status_code, Vec::new(), None));
        };
        status_code.get_or_insert(page_status);
        if !(200..300).contains(&page_status) {
            return Ok((Some(page_status), page_bytes, None));
        }
        let page_value = match serde_json::from_slice::<Value>(&page_bytes) {
            Ok(value) => value,
            Err(_) => return Ok((Some(page_status), page_bytes, None)),
        };
        let Some(page_records) = page_value.as_array() else {
            return Ok((Some(page_status), page_bytes, None));
        };
        records.extend(page_records.iter().cloned());
        next_url = next_from_header;
    }
    let combined_bytes = serde_json::to_vec_pretty(&records).map_err(|err| {
        PolyError::raw_source(
            "POLY_RAW_SOURCE_HF_TREE_JSON_ENCODE_FAILED",
            format!("encode combined Hugging Face tree body for {name}: {err}"),
        )
    })?;
    if combined_bytes.len() as u64 > max_body_bytes {
        return Err(PolyError::raw_source(
            "POLY_RAW_SOURCE_HF_TREE_BODY_LIMIT_EXCEEDED",
            format!(
                "combined Hugging Face tree body for {name} is {} bytes, limit is {max_body_bytes}",
                combined_bytes.len()
            ),
        ));
    }
    Ok((status_code, combined_bytes, None))
}

fn next_hf_tree_url(link_header: &str) -> Option<String> {
    link_header.split(',').find_map(|part| {
        let mut sections = part.split(';');
        let url_part = sections.next()?.trim();
        let is_next = sections.any(|section| {
            let section = section.trim();
            section.eq_ignore_ascii_case("rel=\"next\"") || section.eq_ignore_ascii_case("rel=next")
        });
        if !is_next {
            return None;
        }
        let url = url_part
            .strip_prefix('<')?
            .strip_suffix('>')?
            .trim()
            .to_string();
        if url.starts_with("https://") || url.starts_with("http://") {
            Some(url)
        } else if url.starts_with('/') {
            Some(format!("https://huggingface.co{url}"))
        } else {
            None
        }
    })
}

pub(crate) fn validate_body(format: HistoricalFormat, bytes: &[u8]) -> HistoricalValidation {
    match format {
        HistoricalFormat::Json | HistoricalFormat::HfTreeJson => validate_json(bytes),
        HistoricalFormat::Jsonl => validate_jsonl(bytes),
        HistoricalFormat::Text | HistoricalFormat::Binary => HistoricalValidation {
            ok: !bytes.is_empty(),
            record_count: (!bytes.is_empty()).then_some(1),
            top_level_fields: Vec::new(),
            error_message: bytes
                .is_empty()
                .then(|| "historical body is empty".to_string()),
        },
    }
}

fn validate_json(bytes: &[u8]) -> HistoricalValidation {
    match serde_json::from_slice::<Value>(bytes) {
        Ok(value) => {
            let (record_count, top_level_fields) = json_shape(&value);
            HistoricalValidation {
                ok: true,
                record_count,
                top_level_fields,
                error_message: None,
            }
        }
        Err(err) => HistoricalValidation {
            ok: false,
            record_count: None,
            top_level_fields: Vec::new(),
            error_message: Some(format!("JSON validation failed: {err}")),
        },
    }
}

fn validate_jsonl(bytes: &[u8]) -> HistoricalValidation {
    let text = match std::str::from_utf8(bytes) {
        Ok(text) => text,
        Err(err) => {
            return HistoricalValidation {
                ok: false,
                record_count: None,
                top_level_fields: Vec::new(),
                error_message: Some(format!("JSONL UTF-8 validation failed: {err}")),
            };
        }
    };
    let mut count = 0usize;
    let mut top_level_fields = Vec::new();
    for (index, line) in text
        .lines()
        .filter(|line| !line.trim().is_empty())
        .enumerate()
    {
        let value = match serde_json::from_str::<Value>(line) {
            Ok(value) => value,
            Err(err) => {
                return HistoricalValidation {
                    ok: false,
                    record_count: Some(count),
                    top_level_fields,
                    error_message: Some(format!(
                        "JSONL line {} validation failed: {err}",
                        index + 1
                    )),
                };
            }
        };
        if count == 0 {
            top_level_fields = value
                .as_object()
                .map(|map| map.keys().cloned().collect())
                .unwrap_or_default();
        }
        count += 1;
    }
    HistoricalValidation {
        ok: count > 0,
        record_count: Some(count),
        top_level_fields,
        error_message: (count == 0).then(|| "JSONL body has no records".to_string()),
    }
}

fn json_shape(value: &Value) -> (Option<usize>, Vec<String>) {
    match value {
        Value::Array(items) => (
            Some(items.len()),
            items
                .first()
                .and_then(Value::as_object)
                .map(|map| map.keys().cloned().collect())
                .unwrap_or_default(),
        ),
        Value::Object(map) => (Some(1), map.keys().cloned().collect()),
        Value::Null => (Some(0), Vec::new()),
        _ => (Some(1), Vec::new()),
    }
}

pub(crate) fn persist_body(path: &Path, bytes: &[u8]) -> Result<()> {
    if bytes.is_empty() {
        remove_stale(path)?;
        return Ok(());
    }
    fs::write(path, bytes).map_err(|err| {
        PolyError::raw_source(
            "POLY_RAW_SOURCE_HISTORICAL_BODY_WRITE_FAILED",
            format!(
                "write historical body {}: {err}; expected_sha256={}",
                path.display(),
                sha256_hex(bytes)
            ),
        )
    })
}

pub(crate) fn remove_stale_request(sample_dir: &Path) -> Result<()> {
    remove_stale(&sample_dir.join("request.json"))
}

fn remove_stale(path: &Path) -> Result<()> {
    if path.exists() {
        fs::remove_file(path).map_err(|err| {
            PolyError::raw_source(
                "POLY_RAW_SOURCE_HISTORICAL_STALE_FILE_REMOVE_FAILED",
                format!("remove stale historical file {}: {err}", path.display()),
            )
        })?;
    }
    Ok(())
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum HistoricalFormat {
    Json,
    HfTreeJson,
    Jsonl,
    Text,
    Binary,
}

impl HistoricalFormat {
    pub(crate) fn body_file(self) -> &'static str {
        match self {
            HistoricalFormat::Json | HistoricalFormat::HfTreeJson => "body.json",
            HistoricalFormat::Jsonl => "body.jsonl",
            HistoricalFormat::Text => "body.txt",
            HistoricalFormat::Binary => "body.bin",
        }
    }
}

pub(crate) struct HistoricalValidation {
    pub(crate) ok: bool,
    pub(crate) record_count: Option<usize>,
    pub(crate) top_level_fields: Vec<String>,
    pub(crate) error_message: Option<String>,
}
