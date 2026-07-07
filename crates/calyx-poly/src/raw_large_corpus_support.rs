use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use serde_json::Value;

use crate::raw_large_corpus::{
    LargeCorpusBoundedIncompleteDataset, LargeCorpusEdgeCase, LargeCorpusFailure, LargeCorpusPage,
    LargeCorpusRequest,
};
use crate::raw_large_corpus_profile::{
    CorpusRecord, LargeCorpusFieldProfile, LargeCorpusJoinProfile,
};
use crate::{PolyError, Result};

#[derive(Debug, Clone, Copy)]
pub(crate) enum DatasetPagination {
    Offset,
    Keyset { items_field: &'static str },
}

pub(crate) struct DatasetPlan {
    pub(crate) name: &'static str,
    pub(crate) source: &'static str,
    pub(crate) endpoint: &'static str,
    pub(crate) docs_url: &'static str,
    pub(crate) url: &'static str,
    pub(crate) pagination: DatasetPagination,
}

pub(crate) fn dataset_plans() -> Vec<DatasetPlan> {
    vec![
        DatasetPlan {
            name: "gamma_markets_active_large",
            source: "gamma",
            endpoint: "markets",
            docs_url: "https://docs.polymarket.com/api-reference/markets/list-markets-keyset-pagination",
            url: "https://gamma-api.polymarket.com/markets/keyset?active=true&closed=false",
            pagination: DatasetPagination::Keyset {
                items_field: "markets",
            },
        },
        DatasetPlan {
            name: "gamma_markets_closed_large",
            source: "gamma",
            endpoint: "markets",
            docs_url: "https://docs.polymarket.com/api-reference/markets/list-markets-keyset-pagination",
            url: "https://gamma-api.polymarket.com/markets/keyset?closed=true",
            pagination: DatasetPagination::Keyset {
                items_field: "markets",
            },
        },
        DatasetPlan {
            name: "gamma_events_active_large",
            source: "gamma",
            endpoint: "events",
            docs_url: "https://docs.polymarket.com/api-reference/events/list-events-keyset-pagination",
            url: "https://gamma-api.polymarket.com/events/keyset?active=true&closed=false",
            pagination: DatasetPagination::Keyset {
                items_field: "events",
            },
        },
        DatasetPlan {
            name: "gamma_events_closed_large",
            source: "gamma",
            endpoint: "events",
            docs_url: "https://docs.polymarket.com/api-reference/events/list-events-keyset-pagination",
            url: "https://gamma-api.polymarket.com/events/keyset?closed=true",
            pagination: DatasetPagination::Keyset {
                items_field: "events",
            },
        },
        DatasetPlan {
            name: "data_trades_large",
            source: "data-api",
            endpoint: "trades",
            docs_url: "https://docs.polymarket.com/api-reference/core/get-trades-for-a-user-or-markets",
            url: "https://data-api.polymarket.com/trades",
            pagination: DatasetPagination::Offset,
        },
    ]
}

pub(crate) fn validate_large_corpus_request(request: &LargeCorpusRequest) -> Result<()> {
    if request.output_root.as_os_str().is_empty() {
        return Err(PolyError::raw_source(
            "POLY_LARGE_CORPUS_OUTPUT_ROOT_EMPTY",
            "large corpus output root must not be empty",
        ));
    }
    if request.timeout_secs == 0 {
        return Err(PolyError::raw_source(
            "POLY_LARGE_CORPUS_TIMEOUT_INVALID",
            "large corpus timeout must be greater than zero",
        ));
    }
    if request.max_body_bytes == 0 {
        return Err(PolyError::raw_source(
            "POLY_LARGE_CORPUS_BODY_LIMIT_INVALID",
            "large corpus max body bytes must be greater than zero",
        ));
    }
    if request.page_size == 0 || request.page_size > 500 {
        return Err(PolyError::raw_source(
            "POLY_LARGE_CORPUS_PAGE_SIZE_INVALID",
            format!("page size must be in 1..=500, got {}", request.page_size),
        ));
    }
    if request.max_pages_per_dataset == 0 {
        return Err(PolyError::raw_source(
            "POLY_LARGE_CORPUS_PAGE_LIMIT_INVALID",
            "max pages per dataset must be greater than zero",
        ));
    }
    Ok(())
}

pub(crate) fn page_url(base: &str, limit: usize, offset: usize) -> String {
    let separator = if base.contains('?') { '&' } else { '?' };
    format!("{base}{separator}limit={limit}&offset={offset}")
}

pub(crate) fn keyset_page_url(base: &str, limit: usize, after_cursor: Option<&str>) -> String {
    let separator = if base.contains('?') { '&' } else { '?' };
    let mut url = format!("{base}{separator}limit={limit}");
    if let Some(cursor) = after_cursor {
        url.push_str("&after_cursor=");
        push_query_encoded(&mut url, cursor);
    }
    url
}

fn push_query_encoded(output: &mut String, value: &str) {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                output.push(char::from(byte));
            }
            _ => {
                output.push('%');
                output.push(char::from(HEX[(byte >> 4) as usize]));
                output.push(char::from(HEX[(byte & 0x0f) as usize]));
            }
        }
    }
}

pub(crate) fn collect_records(
    dataset: &str,
    source: &str,
    path: &Path,
    records: &mut Vec<CorpusRecord>,
) -> Result<()> {
    let bytes = fs::read(path).map_err(|err| {
        PolyError::raw_source(
            "POLY_LARGE_CORPUS_RECORD_BODY_READ_FAILED",
            format!("read records from {}: {err}", path.display()),
        )
    })?;
    let value = serde_json::from_slice::<Value>(&bytes).map_err(|err| {
        PolyError::raw_source(
            "POLY_LARGE_CORPUS_RECORD_BODY_DECODE_FAILED",
            format!("decode records from {}: {err}", path.display()),
        )
    })?;
    for value in records_from_value(&value) {
        records.push(CorpusRecord {
            dataset: dataset.to_string(),
            source: source.to_string(),
            value,
        });
    }
    Ok(())
}

pub(crate) fn records_from_value(value: &Value) -> Vec<Value> {
    if let Some(items) = value.as_array() {
        return items.clone();
    }
    if let Some(items) = value.get("json_values").and_then(Value::as_array) {
        return flatten_record_items(items);
    }
    if let Some(items) = value.get("result").and_then(Value::as_array) {
        return items.clone();
    }
    if let Some(records) = records_from_graphql_data(value) {
        return records;
    }
    for field in ["data", "markets", "events", "trades"] {
        if let Some(items) = value.get(field).and_then(Value::as_array) {
            return items.clone();
        }
    }
    value
        .as_object()
        .map(|_| vec![value.clone()])
        .unwrap_or_default()
}

fn flatten_record_items(items: &[Value]) -> Vec<Value> {
    let mut records = Vec::new();
    for item in items {
        if let Some(nested) = item.as_array() {
            records.extend(nested.iter().cloned());
        } else {
            records.push(item.clone());
        }
    }
    records
}

fn records_from_graphql_data(value: &Value) -> Option<Vec<Value>> {
    let data = value.get("data")?.as_object()?;
    let records = data
        .values()
        .filter_map(Value::as_array)
        .flat_map(|items| items.iter().cloned())
        .collect::<Vec<_>>();
    (!records.is_empty()).then_some(records)
}

pub(crate) fn record_count(value: &Value) -> usize {
    records_from_value(value).len()
}

pub(crate) fn manifest_failure(
    pages: &[LargeCorpusPage],
    edges: &[LargeCorpusEdgeCase],
    profiles: &[LargeCorpusFieldProfile],
    join_profile: &LargeCorpusJoinProfile,
    require_exhaustive: bool,
    bounded_incomplete_datasets: &[LargeCorpusBoundedIncompleteDataset],
) -> Option<LargeCorpusFailure> {
    if let Some(page) = pages.iter().find(|page| !page.expectation_met) {
        return Some(failure(
            "POLY_LARGE_CORPUS_PAGE_EXPECTATION_FAILED",
            format!("large corpus page {} failed expectation", page.body_path),
        ));
    }
    if let Some(edge) = edges.iter().find(|edge| !edge.expectation_met) {
        return Some(failure(
            "POLY_LARGE_CORPUS_EDGE_EXPECTATION_FAILED",
            format!("large corpus edge {} failed expectation", edge.name),
        ));
    }
    for plan in dataset_plans() {
        if !pages
            .iter()
            .any(|page| page.dataset == plan.name && page.record_count > 0)
        {
            return Some(failure(
                "POLY_LARGE_CORPUS_DATASET_EMPTY",
                format!("dataset {} produced no records", plan.name),
            ));
        }
    }
    if let Some(failure) =
        exhaustive_incomplete_failure(require_exhaustive, bounded_incomplete_datasets)
    {
        return Some(failure);
    }
    if profiles.is_empty() {
        return Some(failure(
            "POLY_LARGE_CORPUS_FIELD_PROFILES_EMPTY",
            "field profiles were not generated from captured records",
        ));
    }
    if join_profile.identifier_counts.is_empty() {
        return Some(failure(
            "POLY_LARGE_CORPUS_JOIN_PROFILE_EMPTY",
            "join profile has no observed identifiers",
        ));
    }
    None
}

pub(crate) fn bounded_incomplete_datasets(
    pages: &[LargeCorpusPage],
    page_size: usize,
    max_pages_per_dataset: usize,
) -> Vec<LargeCorpusBoundedIncompleteDataset> {
    let mut grouped: BTreeMap<&str, Vec<&LargeCorpusPage>> = BTreeMap::new();
    for page in pages {
        grouped.entry(page.dataset.as_str()).or_default().push(page);
    }
    let mut incomplete = Vec::new();
    for dataset_pages in grouped.values_mut() {
        dataset_pages.sort_by_key(|page| page.page_index);
        let Some(last) = dataset_pages.last() else {
            continue;
        };
        let source_specific_reason = source_specific_incomplete_reason(last);
        if let Some(reason) = source_specific_reason {
            incomplete.push(incomplete_dataset(
                last,
                dataset_pages.len(),
                page_size,
                reason,
            ));
        } else if dataset_pages.len() >= max_pages_per_dataset
            && last.stop_reason.is_none()
            && last.record_count >= page_size
        {
            incomplete.push(incomplete_dataset(
                last,
                dataset_pages.len(),
                page_size,
                "page_limit_reached_without_terminal_page",
            ));
        }
    }
    incomplete
}

fn source_specific_incomplete_reason(page: &LargeCorpusPage) -> Option<&'static str> {
    if page.dataset == "data_trades_large" {
        return Some("data_api_global_trades_is_bounded_activity_window_not_all_time_source");
    }
    if page.source == "polygon-rpc"
        && page.dataset.contains("order_filled_chunked")
        && page.range_state.is_some()
    {
        return Some("polygon_order_filled_recent_block_window_not_deployment_to_latest");
    }
    None
}

fn incomplete_dataset(
    page: &LargeCorpusPage,
    page_count: usize,
    page_size: usize,
    reason: &str,
) -> LargeCorpusBoundedIncompleteDataset {
    LargeCorpusBoundedIncompleteDataset {
        dataset: page.dataset.clone(),
        source: page.source.clone(),
        endpoint: page.endpoint.clone(),
        page_count,
        last_page_index: page.page_index,
        last_record_count: page.record_count,
        page_size,
        reason: reason.to_string(),
    }
}

pub(crate) fn exhaustive_incomplete_failure(
    require_exhaustive: bool,
    bounded_incomplete_datasets: &[LargeCorpusBoundedIncompleteDataset],
) -> Option<LargeCorpusFailure> {
    if !require_exhaustive || bounded_incomplete_datasets.is_empty() {
        return None;
    }
    let names = bounded_incomplete_datasets
        .iter()
        .take(8)
        .map(|dataset| dataset.dataset.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    Some(failure(
        "POLY_LARGE_CORPUS_EXHAUSTIVE_INCOMPLETE",
        format!(
            "{} datasets hit the configured page cap without terminal exhaustion: {names}",
            bounded_incomplete_datasets.len()
        ),
    ))
}

pub(crate) fn capture_goal(require_exhaustive: bool) -> &'static str {
    if require_exhaustive {
        "exhaustive_backfill"
    } else {
        "representative_large_corpus"
    }
}

pub(crate) fn failure(code: impl Into<String>, message: impl Into<String>) -> LargeCorpusFailure {
    LargeCorpusFailure {
        code: code.into(),
        message: message.into(),
    }
}
