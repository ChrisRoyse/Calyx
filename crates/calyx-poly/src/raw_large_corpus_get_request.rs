use std::fs;
use std::path::Path;

use serde_json::{Value, json};

use crate::raw_large_corpus::{LargeCorpusPaginationState, LargeCorpusRequest};
use crate::raw_large_corpus_support::{DatasetPagination, DatasetPlan};
use crate::raw_source_support::sha256_hex;
use crate::{PolyError, Result};

pub(crate) fn persist_get_request(
    path: &Path,
    plan: &DatasetPlan,
    url: &str,
    request: &LargeCorpusRequest,
    requested_offset: Option<usize>,
    request_after_cursor: Option<&str>,
) -> Result<(Vec<u8>, Option<LargeCorpusPaginationState>)> {
    let (mode, items_field) = match plan.pagination {
        DatasetPagination::Offset => ("offset", None),
        DatasetPagination::Keyset { items_field } => ("keyset", Some(items_field)),
    };
    let state = LargeCorpusPaginationState {
        mode: mode.to_string(),
        items_field: items_field.map(ToString::to_string),
        requested_limit: request.page_size,
        requested_offset,
        request_after_cursor: request_after_cursor.map(ToString::to_string),
        response_next_cursor: None,
        terminal: false,
    };
    let request_doc = json!({
        "method": "GET",
        "url": url,
        "source": plan.source,
        "endpoint": plan.endpoint,
        "dataset": plan.name,
        "docs_url": plan.docs_url,
        "pagination": &state
    });
    let bytes = serde_json::to_vec(&request_doc).map_err(|err| {
        PolyError::raw_source(
            "POLY_LARGE_CORPUS_GET_REQUEST_ENCODE_FAILED",
            format!("encode GET request {}: {err}", path.display()),
        )
    })?;
    fs::write(path, &bytes).map_err(|err| {
        PolyError::raw_source(
            "POLY_LARGE_CORPUS_GET_REQUEST_WRITE_FAILED",
            format!("write GET request {}: {err}", path.display()),
        )
    })?;
    assert_request_readback(path, &bytes)?;
    Ok((bytes, Some(state)))
}

pub(crate) fn keyset_item_count(value: &Value, items_field: &str) -> Option<usize> {
    value
        .get(items_field)
        .and_then(Value::as_array)
        .map(Vec::len)
}

pub(crate) fn keyset_next_cursor(value: &Value) -> Option<&str> {
    value
        .get("next_cursor")
        .and_then(Value::as_str)
        .filter(|cursor| !cursor.is_empty())
}

fn assert_request_readback(path: &Path, expected: &[u8]) -> Result<()> {
    let readback = fs::read(path).map_err(|err| {
        PolyError::raw_source(
            "POLY_LARGE_CORPUS_GET_REQUEST_READBACK_FAILED",
            format!("read back GET request {}: {err}", path.display()),
        )
    })?;
    if readback != expected {
        return Err(PolyError::raw_source(
            "POLY_LARGE_CORPUS_GET_REQUEST_READBACK_MISMATCH",
            format!(
                "GET request readback mismatch at {}; expected_sha256={} actual_sha256={}",
                path.display(),
                sha256_hex(expected),
                sha256_hex(&readback)
            ),
        ));
    }
    Ok(())
}
