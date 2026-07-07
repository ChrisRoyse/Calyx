//! Large representative raw Polymarket corpus capture for schema-lock evidence.

use std::fs;
use std::path::Path;
use std::time::Duration;

use serde_json::Value;

use crate::rate_limit_governor::{
    RateLimitEndpoint, RateLimitedHttpOutcome, execute_rate_limited_request, parse_retry_after_ms,
};
use crate::raw_large_corpus_clob::{capture_clob_edge_cases, capture_clob_market_data};
use crate::raw_large_corpus_clob_plan::derive_clob_targets;
pub use crate::raw_large_corpus_failure::LargeCorpusFailure;
use crate::raw_large_corpus_get_request::{
    keyset_item_count, keyset_next_cursor, persist_get_request,
};
use crate::raw_large_corpus_historical::{
    capture_historical_edge_cases, capture_historical_market_data,
};
use crate::raw_large_corpus_onchain::{capture_onchain_edge_cases, capture_onchain_market_data};
use crate::raw_large_corpus_onchain_backfill::write_onchain_backfill_state;
use crate::raw_large_corpus_profile::{
    CorpusRecord, build_field_profiles, build_join_profile, write_field_profiles,
};
pub use crate::raw_large_corpus_range::LargeCorpusRangeState;
pub use crate::raw_large_corpus_readback::{
    read_large_corpus_manifest, readback_large_corpus, readback_large_corpus_with_exhaustive,
    require_large_corpus_passed,
};
use crate::raw_large_corpus_schema_note::{BackfillSchemaStates, write_schema_decision_input};
use crate::raw_large_corpus_support::{
    DatasetPagination, DatasetPlan, bounded_incomplete_datasets, capture_goal, collect_records,
    dataset_plans, keyset_page_url, manifest_failure, page_url, record_count,
};
use crate::raw_large_corpus_trade_history::write_trade_history_source_state;
pub use crate::raw_large_corpus_types::{
    LargeCorpusBoundedIncompleteDataset, LargeCorpusEdgeCase, LargeCorpusManifest, LargeCorpusPage,
    LargeCorpusPaginationState, LargeCorpusReadbackReport, LargeCorpusRequest,
};
use crate::raw_large_corpus_websocket::{
    capture_websocket_edge_cases, capture_websocket_market_data,
};
use crate::raw_large_corpus_ws_semantics::{
    build_websocket_runtime_semantics, write_websocket_runtime_semantics,
};
use crate::raw_source_support::{file_state, now_unix_ms, sha256_hex, write_json};
use crate::raw_sources::RawFileState;
use crate::{PolyError, Result};

pub const LARGE_CORPUS_SCHEMA_VERSION: &str = "poly.large_corpus.v1";
pub const LARGE_CORPUS_CAPTURE_PASSED: &str = "POLY_LARGE_CORPUS_CAPTURE_PASSED";

pub fn run_large_corpus_capture(request: LargeCorpusRequest) -> Result<LargeCorpusManifest> {
    let request = request.normalized()?;
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(request.timeout_secs)))
        .http_status_as_error(false)
        .build()
        .into();
    let mut pages = Vec::new();
    let mut records = Vec::new();
    for plan in dataset_plans() {
        capture_dataset(&request, &agent, &plan, &mut pages, &mut records)?;
    }
    let clob_targets = derive_clob_targets(&records, request.max_pages_per_dataset * 2);
    capture_clob_market_data(&request, &agent, &clob_targets, &mut pages, &mut records)?;
    capture_websocket_market_data(&request, &clob_targets, &mut pages, &mut records)?;
    capture_historical_market_data(&request, &agent, &mut pages, &mut records)?;
    let latest_onchain_block =
        capture_onchain_market_data(&request, &agent, &mut pages, &mut records)?;
    let mut edge_cases = capture_edge_cases(&request, &agent)?;
    edge_cases.extend(capture_clob_edge_cases(&request, &agent, &clob_targets)?);
    edge_cases.extend(capture_websocket_edge_cases(&request, &clob_targets)?);
    edge_cases.extend(capture_historical_edge_cases(&request, &agent)?);
    edge_cases.extend(capture_onchain_edge_cases(
        &request,
        &agent,
        latest_onchain_block,
    )?);
    let field_profiles = build_field_profiles(&records);
    let field_profile_paths = write_field_profiles(&request.output_root, &field_profiles)?;
    let join_profile = build_join_profile(&records);
    let join_profile_path = request.output_root.join("join-profile.json");
    write_json(&join_profile_path, &join_profile)?;
    let websocket_runtime_semantics = build_websocket_runtime_semantics(&pages, &edge_cases)?;
    write_websocket_runtime_semantics(&request.output_root, &websocket_runtime_semantics)?;
    let (trade_history_state_path, trade_history_state) =
        write_trade_history_source_state(&request.output_root, &pages, &edge_cases)?;
    let (onchain_backfill_state_path, onchain_backfill_state) =
        write_onchain_backfill_state(&request, &agent, latest_onchain_block, &pages)?;
    let bounded_incomplete_datasets =
        bounded_incomplete_datasets(&pages, request.page_size, request.max_pages_per_dataset);
    let schema_decision_input_path = write_schema_decision_input(
        &request.output_root,
        &field_profiles,
        &join_profile,
        &pages,
        &websocket_runtime_semantics.observations,
        BackfillSchemaStates {
            trade_history: &trade_history_state,
            onchain: &onchain_backfill_state,
        },
        &bounded_incomplete_datasets,
    )?;
    let failure = manifest_failure(
        &pages,
        &edge_cases,
        &field_profiles,
        &join_profile,
        request.require_exhaustive,
        &bounded_incomplete_datasets,
    )
    .or(websocket_runtime_semantics.failure.clone());
    let passed = failure.is_none();
    let manifest = LargeCorpusManifest {
        schema_version: LARGE_CORPUS_SCHEMA_VERSION.to_string(),
        captured_at_unix_ms: now_unix_ms()?,
        source_of_truth:
            "live public/read-only Polymarket API responses plus physical raw corpus files"
                .to_string(),
        capture_goal: capture_goal(request.require_exhaustive).to_string(),
        require_exhaustive: request.require_exhaustive,
        page_size: request.page_size,
        max_pages_per_dataset: request.max_pages_per_dataset,
        bounded_incomplete_datasets,
        trade_history_state_path: trade_history_state_path.display().to_string(),
        onchain_backfill_state_path: onchain_backfill_state_path.display().to_string(),
        total_pages: pages.len(),
        total_records: pages.iter().map(|page| page.record_count).sum(),
        total_body_bytes: pages.iter().map(|page| page.body_bytes).sum(),
        pages,
        edge_cases,
        field_profile_paths,
        join_profile_path: join_profile_path.display().to_string(),
        schema_decision_input_path: schema_decision_input_path.display().to_string(),
        status_code: if passed {
            LARGE_CORPUS_CAPTURE_PASSED.to_string()
        } else {
            failure
                .as_ref()
                .map(|failure| failure.code.clone())
                .unwrap_or_else(|| "POLY_LARGE_CORPUS_CAPTURE_FAILED".to_string())
        },
        passed,
        failure,
    };
    write_json(
        &request.output_root.join("large-corpus-manifest.json"),
        &manifest,
    )?;
    Ok(manifest)
}

fn capture_dataset(
    request: &LargeCorpusRequest,
    agent: &ureq::Agent,
    plan: &DatasetPlan,
    pages: &mut Vec<LargeCorpusPage>,
    records: &mut Vec<CorpusRecord>,
) -> Result<()> {
    let mut after_cursor: Option<String> = None;
    for page_index in 0..request.max_pages_per_dataset {
        let (url, offset) = match plan.pagination {
            DatasetPagination::Offset => {
                let offset = page_index * request.page_size;
                (page_url(plan.url, request.page_size, offset), Some(offset))
            }
            DatasetPagination::Keyset { .. } => (
                keyset_page_url(plan.url, request.page_size, after_cursor.as_deref()),
                None,
            ),
        };
        let page = capture_page(
            request,
            agent,
            plan,
            page_index,
            &url,
            offset,
            after_cursor.as_deref(),
        )?;
        collect_records(
            &page.dataset,
            &page.source,
            Path::new(&page.body_path),
            records,
        )?;
        let should_stop = page.stop_reason.is_some();
        after_cursor = page
            .pagination_state
            .as_ref()
            .and_then(|state| state.response_next_cursor.clone());
        pages.push(page);
        if should_stop {
            break;
        }
    }
    if !pages.iter().any(|page| page.dataset == plan.name) {
        return Err(PolyError::raw_source(
            "POLY_LARGE_CORPUS_DATASET_UNSAMPLED",
            format!("dataset {} produced no persisted page", plan.name),
        ));
    }
    Ok(())
}

fn capture_page(
    request: &LargeCorpusRequest,
    agent: &ureq::Agent,
    plan: &DatasetPlan,
    page_index: usize,
    url: &str,
    requested_offset: Option<usize>,
    request_after_cursor: Option<&str>,
) -> Result<LargeCorpusPage> {
    let dir = request.output_root.join("raw").join(plan.name);
    let body_path = dir.join(format!("page-{page_index:06}.json"));
    let metadata_path = dir.join(format!("page-{page_index:06}.metadata.json"));
    let request_path = dir.join(format!("page-{page_index:06}.request.json"));
    let before = file_state(&body_path, &metadata_path)?;
    fs::create_dir_all(&dir).map_err(|err| {
        PolyError::raw_source(
            "POLY_LARGE_CORPUS_PAGE_DIR_CREATE_FAILED",
            format!("create large corpus page dir {}: {err}", dir.display()),
        )
    })?;
    let (request_bytes, mut pagination_state) = persist_get_request(
        &request_path,
        plan,
        url,
        request,
        requested_offset,
        request_after_cursor,
    )?;
    let (status_code, bytes) = get_bytes(
        agent,
        plan.source,
        plan.endpoint,
        url,
        request.max_body_bytes,
    )?;
    fs::write(&body_path, &bytes).map_err(|err| {
        PolyError::raw_source(
            "POLY_LARGE_CORPUS_PAGE_BODY_WRITE_FAILED",
            format!("write page body {}: {err}", body_path.display()),
        )
    })?;
    let parsed = serde_json::from_slice::<Value>(&bytes);
    let record_count = match plan.pagination {
        DatasetPagination::Offset => parsed.as_ref().map(record_count).unwrap_or(0),
        DatasetPagination::Keyset { items_field } => parsed
            .as_ref()
            .ok()
            .and_then(|value| keyset_item_count(value, items_field))
            .unwrap_or(0),
    };
    let http_success = status_code.is_some_and(|code| (200..300).contains(&code));
    let response_next_cursor = parsed
        .as_ref()
        .ok()
        .and_then(keyset_next_cursor)
        .map(ToString::to_string);
    if let Some(state) = &mut pagination_state {
        state.response_next_cursor.clone_from(&response_next_cursor);
        state.terminal = response_next_cursor.is_none();
    }
    let stop_reason = match plan.pagination {
        DatasetPagination::Offset => (record_count < request.page_size).then(|| {
            if record_count == 0 {
                "empty_page"
            } else {
                "short_page"
            }
            .to_string()
        }),
        DatasetPagination::Keyset { .. } => {
            if record_count == 0 {
                Some("empty_page".to_string())
            } else if response_next_cursor.is_none() {
                Some("terminal_cursor_absent".to_string())
            } else {
                None
            }
        }
    };
    let pagination_expectation_met = match plan.pagination {
        DatasetPagination::Offset => true,
        DatasetPagination::Keyset { items_field } => parsed.as_ref().is_ok_and(|value| {
            value.get(items_field).and_then(Value::as_array).is_some()
                && !(record_count == 0 && response_next_cursor.is_some())
        }),
    };
    let mut page = LargeCorpusPage {
        dataset: plan.name.to_string(),
        source: plan.source.to_string(),
        endpoint: plan.endpoint.to_string(),
        method: "GET".to_string(),
        docs_url: plan.docs_url.to_string(),
        page_index,
        url: url.to_string(),
        request_path: Some(request_path.display().to_string()),
        request_body_bytes: request_bytes.len() as u64,
        request_body_sha256: Some(sha256_hex(&request_bytes)),
        status_code,
        http_success,
        expectation_met: http_success
            && parsed.is_ok()
            && !bytes.is_empty()
            && pagination_expectation_met,
        record_count,
        stop_reason,
        body_path: body_path.display().to_string(),
        metadata_path: metadata_path.display().to_string(),
        body_format: "json".to_string(),
        body_bytes: bytes.len() as u64,
        body_sha256: Some(sha256_hex(&bytes)),
        json_parse_ok: parsed.is_ok(),
        websocket_frame_count: None,
        websocket_json_frame_count: None,
        websocket_event_types: Vec::new(),
        no_payload_window: false,
        pagination_state,
        range_state: None,
        before,
        after: empty_state(),
    };
    write_json(&metadata_path, &page)?;
    page.after = file_state(&body_path, &metadata_path)?;
    if page.body_sha256 != page.after.body_sha256 {
        page.expectation_met = false;
    }
    write_json(&metadata_path, &page)?;
    Ok(page)
}

fn capture_edge_cases(
    request: &LargeCorpusRequest,
    agent: &ureq::Agent,
) -> Result<Vec<LargeCorpusEdgeCase>> {
    let edges = [
        (
            "edge_gamma_markets_missing_slug_empty",
            "https://gamma-api.polymarket.com/markets?slug=poly-fsv-nonexistent-20260704&limit=100",
            "empty_success_page",
        ),
        (
            "edge_data_trades_offset_cap_rejected",
            "https://data-api.polymarket.com/trades?limit=3&offset=10000",
            "expected_http_failure",
        ),
        (
            "edge_gamma_markets_keyset_offset_rejected",
            "https://gamma-api.polymarket.com/markets/keyset?limit=100&offset=1",
            "expected_http_failure",
        ),
        (
            "edge_gamma_markets_keyset_invalid_cursor_rejected",
            "https://gamma-api.polymarket.com/markets/keyset?limit=3&after_cursor=poly-invalid-cursor",
            "expected_http_failure",
        ),
    ];
    edges
        .iter()
        .map(|(name, url, semantics)| capture_edge_case(request, agent, name, url, semantics))
        .collect()
}

fn capture_edge_case(
    request: &LargeCorpusRequest,
    agent: &ureq::Agent,
    name: &str,
    url: &str,
    semantics: &str,
) -> Result<LargeCorpusEdgeCase> {
    let dir = request.output_root.join("edge").join(name);
    let body_path = dir.join("body.json");
    let metadata_path = dir.join("metadata.json");
    let before = file_state(&body_path, &metadata_path)?;
    let (status_code, bytes) = get_bytes(
        agent,
        "large-corpus-edge",
        name,
        url,
        request.max_body_bytes,
    )?;
    fs::create_dir_all(&dir).map_err(|err| {
        PolyError::raw_source(
            "POLY_LARGE_CORPUS_EDGE_DIR_CREATE_FAILED",
            format!("create edge dir {}: {err}", dir.display()),
        )
    })?;
    fs::write(&body_path, &bytes).map_err(|err| {
        PolyError::raw_source(
            "POLY_LARGE_CORPUS_EDGE_BODY_WRITE_FAILED",
            format!("write edge body {}: {err}", body_path.display()),
        )
    })?;
    let parsed = serde_json::from_slice::<Value>(&bytes).ok();
    let record_count = parsed.as_ref().map(record_count).unwrap_or(0);
    let http_success = status_code.is_some_and(|code| (200..300).contains(&code));
    let expectation_met = match semantics {
        "empty_success_page" => http_success && record_count == 0,
        "expected_http_failure" => !http_success,
        "defaulted_non_empty_success" => http_success && record_count > 0,
        _ => false,
    };
    let mut edge = LargeCorpusEdgeCase {
        name: name.to_string(),
        method: "GET".to_string(),
        url: url.to_string(),
        request_path: None,
        request_body_bytes: 0,
        request_body_sha256: None,
        expected_semantics: semantics.to_string(),
        status_code,
        expectation_met,
        record_count,
        body_path: body_path.display().to_string(),
        metadata_path: metadata_path.display().to_string(),
        body_format: "json".to_string(),
        json_parse_ok: parsed.is_some(),
        body_sha256: Some(sha256_hex(&bytes)),
        websocket_frame_count: None,
        websocket_json_frame_count: None,
        websocket_event_types: Vec::new(),
        no_payload_window: false,
        range_state: None,
        before,
        after: empty_state(),
    };
    write_json(&metadata_path, &edge)?;
    edge.after = file_state(&body_path, &metadata_path)?;
    if edge.body_sha256 != edge.after.body_sha256 {
        edge.expectation_met = false;
    }
    write_json(&metadata_path, &edge)?;
    Ok(edge)
}

fn get_bytes(
    agent: &ureq::Agent,
    source: &str,
    endpoint_name: &str,
    url: &str,
    limit: usize,
) -> Result<(Option<u16>, Vec<u8>)> {
    let endpoint = RateLimitEndpoint::new(source, endpoint_name, "GET");
    execute_rate_limited_request(&endpoint, || {
        let mut response = agent
            .get(url)
            .header("Accept", "application/json")
            .call()
            .map_err(|err| {
                PolyError::raw_source(
                    "POLY_LARGE_CORPUS_HTTP_TRANSPORT_FAILED",
                    format!("fetch {url}: {err}"),
                )
            })?;
        let status_code = Some(response.status().as_u16());
        let retry_after_ms = parse_retry_after_ms(
            response
                .headers()
                .get("retry-after")
                .and_then(|value| value.to_str().ok()),
        );
        let bytes = response
            .body_mut()
            .with_config()
            .limit(limit as u64)
            .read_to_vec()
            .map_err(|err| {
                PolyError::raw_source(
                    "POLY_LARGE_CORPUS_BODY_READ_FAILED",
                    format!("read body from {url}: {err}"),
                )
            })?;
        Ok(RateLimitedHttpOutcome {
            status_code,
            retry_after_ms,
            value: (status_code, bytes),
        })
    })
}

fn empty_state() -> RawFileState {
    RawFileState {
        body_exists: false,
        metadata_exists: false,
        body_bytes: 0,
        body_sha256: None,
    }
}
