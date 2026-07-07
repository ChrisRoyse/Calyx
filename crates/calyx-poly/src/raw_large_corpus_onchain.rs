use std::fs;
use std::path::Path;

use serde_json::Value;

use crate::rate_limit_governor::{
    RateLimitEndpoint, RateLimitedHttpOutcome, execute_rate_limited_request, parse_retry_after_ms,
};
use crate::raw_large_corpus::{LargeCorpusEdgeCase, LargeCorpusPage, LargeCorpusRequest};
use crate::raw_large_corpus_onchain_chunks::{
    capture_polygon_chunk_edge_cases, capture_polygon_chunked_log_pages,
    collect_polygon_chunk_records,
};
use crate::raw_large_corpus_onchain_plans::{
    PostPlan, onchain_edge_plans, onchain_plans, polygon_block_number_plan,
};
use crate::raw_large_corpus_onchain_specs::{chunk_source_specs, ctf_chunk_spec};
use crate::raw_large_corpus_profile::CorpusRecord;
use crate::raw_large_corpus_support::{record_count, records_from_value};
use crate::raw_source_support::{file_state, sha256_hex, write_json};
use crate::raw_sources::RawFileState;
use crate::{PolyError, Result};

pub(crate) fn capture_onchain_market_data(
    request: &LargeCorpusRequest,
    agent: &ureq::Agent,
    pages: &mut Vec<LargeCorpusPage>,
    records: &mut Vec<CorpusRecord>,
) -> Result<u64> {
    let block_plan = polygon_block_number_plan();
    let block_page = capture_post_page(request, agent, &block_plan)?;
    let latest_block = latest_block_number(Path::new(&block_page.body_path), &block_page.dataset)?;
    collect_json_records(&block_page, records)?;
    pages.push(block_page);

    for plan in onchain_plans(latest_block) {
        let page = capture_post_page(request, agent, &plan)?;
        collect_json_records(&page, records)?;
        pages.push(page);
    }
    for spec in chunk_source_specs() {
        for page in capture_polygon_chunked_log_pages(request, agent, latest_block, spec)? {
            collect_polygon_chunk_records(&page, records)?;
            pages.push(page);
        }
    }
    Ok(latest_block)
}

pub(crate) fn capture_onchain_edge_cases(
    request: &LargeCorpusRequest,
    agent: &ureq::Agent,
    latest_block: u64,
) -> Result<Vec<LargeCorpusEdgeCase>> {
    let mut edges = onchain_edge_plans()
        .iter()
        .map(|plan| capture_post_edge(request, agent, plan))
        .collect::<Result<Vec<_>>>()?;
    edges.extend(capture_polygon_chunk_edge_cases(
        request,
        agent,
        latest_block,
        ctf_chunk_spec(),
    )?);
    Ok(edges)
}

fn capture_post_page(
    request: &LargeCorpusRequest,
    agent: &ureq::Agent,
    plan: &PostPlan,
) -> Result<LargeCorpusPage> {
    let dir = request.output_root.join("raw").join(&plan.dataset);
    let body_path = dir.join("page-000000.json");
    let metadata_path = dir.join("page-000000.metadata.json");
    let request_path = dir.join("page-000000.request.json");
    let before = file_state(&body_path, &metadata_path)?;
    fs::create_dir_all(&dir).map_err(|err| {
        PolyError::raw_source(
            "POLY_LARGE_CORPUS_ONCHAIN_DIR_CREATE_FAILED",
            format!("create on-chain corpus dir {}: {err}", dir.display()),
        )
    })?;
    let request_bytes = persist_request(&request_path, &plan.request_body)?;
    let (status_code, bytes, transport_error) =
        execute_post(agent, plan, &request_bytes, request.max_body_bytes)?;
    fs::write(&body_path, &bytes).map_err(|err| {
        PolyError::raw_source(
            "POLY_LARGE_CORPUS_ONCHAIN_BODY_WRITE_FAILED",
            format!("write on-chain body {}: {err}", body_path.display()),
        )
    })?;
    let parsed = serde_json::from_slice::<Value>(&bytes);
    let http_success = status_code.is_some_and(|code| (200..300).contains(&code));
    let has_error = parsed.as_ref().is_ok_and(has_response_error);
    let mut page = LargeCorpusPage {
        dataset: plan.dataset.clone(),
        source: plan.source.clone(),
        endpoint: plan.endpoint.clone(),
        method: "POST".to_string(),
        docs_url: plan.docs_url.clone(),
        page_index: 0,
        url: plan.url.clone(),
        request_path: Some(request_path.display().to_string()),
        request_body_bytes: request_bytes.len() as u64,
        request_body_sha256: Some(sha256_hex(&request_bytes)),
        status_code,
        http_success,
        expectation_met: http_success
            && parsed.is_ok()
            && !has_error
            && transport_error.is_none()
            && !bytes.is_empty(),
        record_count: parsed.as_ref().map(record_count).unwrap_or(0),
        stop_reason: Some(plan.expected_semantics.clone()),
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
        pagination_state: None,
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

fn capture_post_edge(
    request: &LargeCorpusRequest,
    agent: &ureq::Agent,
    plan: &PostPlan,
) -> Result<LargeCorpusEdgeCase> {
    let dir = request.output_root.join("edge").join(&plan.dataset);
    let body_path = dir.join("body.json");
    let metadata_path = dir.join("metadata.json");
    let request_path = dir.join("request.json");
    let before = file_state(&body_path, &metadata_path)?;
    fs::create_dir_all(&dir).map_err(|err| {
        PolyError::raw_source(
            "POLY_LARGE_CORPUS_ONCHAIN_EDGE_DIR_CREATE_FAILED",
            format!("create on-chain edge dir {}: {err}", dir.display()),
        )
    })?;
    let request_bytes = persist_request(&request_path, &plan.request_body)?;
    let (status_code, bytes, transport_error) =
        execute_post(agent, plan, &request_bytes, request.max_body_bytes)?;
    fs::write(&body_path, &bytes).map_err(|err| {
        PolyError::raw_source(
            "POLY_LARGE_CORPUS_ONCHAIN_EDGE_BODY_WRITE_FAILED",
            format!("write on-chain edge body {}: {err}", body_path.display()),
        )
    })?;
    let parsed = serde_json::from_slice::<Value>(&bytes);
    let has_error = parsed.as_ref().is_ok_and(has_response_error);
    let expectation_met = match plan.expected_semantics.as_str() {
        "expected_json_rpc_error" | "expected_graphql_error" => {
            parsed.is_ok() && has_error && transport_error.is_none()
        }
        _ => false,
    };
    let mut edge = LargeCorpusEdgeCase {
        name: plan.dataset.clone(),
        method: "POST".to_string(),
        url: plan.url.clone(),
        request_path: Some(request_path.display().to_string()),
        request_body_bytes: request_bytes.len() as u64,
        request_body_sha256: Some(sha256_hex(&request_bytes)),
        expected_semantics: plan.expected_semantics.clone(),
        status_code,
        expectation_met,
        record_count: parsed.as_ref().map(record_count).unwrap_or(0),
        body_path: body_path.display().to_string(),
        metadata_path: metadata_path.display().to_string(),
        body_format: "json".to_string(),
        json_parse_ok: parsed.is_ok(),
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

fn persist_request(path: &Path, body: &Value) -> Result<Vec<u8>> {
    let bytes = serde_json::to_vec_pretty(body).map_err(|err| {
        PolyError::raw_source(
            "POLY_LARGE_CORPUS_ONCHAIN_REQUEST_ENCODE_FAILED",
            format!("encode on-chain request {}: {err}", path.display()),
        )
    })?;
    fs::write(path, &bytes).map_err(|err| {
        PolyError::raw_source(
            "POLY_LARGE_CORPUS_ONCHAIN_REQUEST_WRITE_FAILED",
            format!("write on-chain request {}: {err}", path.display()),
        )
    })?;
    let readback = fs::read(path).map_err(|err| {
        PolyError::raw_source(
            "POLY_LARGE_CORPUS_ONCHAIN_REQUEST_READBACK_FAILED",
            format!("read on-chain request {}: {err}", path.display()),
        )
    })?;
    if readback != bytes {
        return Err(PolyError::raw_source(
            "POLY_LARGE_CORPUS_ONCHAIN_REQUEST_READBACK_MISMATCH",
            format!("request readback mismatch at {}", path.display()),
        ));
    }
    Ok(bytes)
}

fn execute_post(
    agent: &ureq::Agent,
    plan: &PostPlan,
    request_body: &[u8],
    max_body_bytes: usize,
) -> Result<(Option<u16>, Vec<u8>, Option<String>)> {
    let endpoint = RateLimitEndpoint::new(&plan.source, &plan.endpoint, "POST");
    execute_rate_limited_request(&endpoint, || {
        let result = agent
            .post(&plan.url)
            .header("Accept", "application/json")
            .header("Content-Type", "application/json")
            .send(request_body);
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
                    .limit(max_body_bytes as u64)
                    .read_to_vec()
                    .map_err(|err| {
                        PolyError::raw_source(
                            "POLY_LARGE_CORPUS_ONCHAIN_BODY_READ_FAILED",
                            format!("read on-chain body for {}: {err}", plan.dataset),
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

fn collect_json_records(page: &LargeCorpusPage, records: &mut Vec<CorpusRecord>) -> Result<()> {
    let bytes = fs::read(Path::new(&page.body_path)).map_err(|err| {
        PolyError::raw_source(
            "POLY_LARGE_CORPUS_ONCHAIN_RECORD_READ_FAILED",
            format!("read on-chain page {}: {err}", page.body_path),
        )
    })?;
    let value = serde_json::from_slice::<Value>(&bytes).map_err(|err| {
        PolyError::raw_source(
            "POLY_LARGE_CORPUS_ONCHAIN_RECORD_DECODE_FAILED",
            format!("decode on-chain page {}: {err}", page.body_path),
        )
    })?;
    for value in records_from_value(&value) {
        records.push(CorpusRecord {
            dataset: page.dataset.clone(),
            source: page.source.clone(),
            value,
        });
    }
    Ok(())
}

fn latest_block_number(path: &Path, dataset: &str) -> Result<u64> {
    let bytes = fs::read(path).map_err(|err| {
        PolyError::raw_source(
            "POLY_LARGE_CORPUS_ONCHAIN_BLOCK_READ_FAILED",
            format!("read block-number body {}: {err}", path.display()),
        )
    })?;
    let value = serde_json::from_slice::<Value>(&bytes).map_err(|err| {
        PolyError::raw_source(
            "POLY_LARGE_CORPUS_ONCHAIN_BLOCK_DECODE_FAILED",
            format!("decode block-number body for {dataset}: {err}"),
        )
    })?;
    let result = value.get("result").and_then(Value::as_str).ok_or_else(|| {
        PolyError::raw_source(
            "POLY_LARGE_CORPUS_ONCHAIN_BLOCK_RESULT_MISSING",
            format!("block-number response for {dataset} missing string result"),
        )
    })?;
    u64::from_str_radix(result.trim_start_matches("0x"), 16).map_err(|err| {
        PolyError::raw_source(
            "POLY_LARGE_CORPUS_ONCHAIN_BLOCK_RESULT_INVALID",
            format!("block-number result {result} is not hex: {err}"),
        )
    })
}

fn has_response_error(value: &Value) -> bool {
    value.get("error").is_some()
        || value
            .get("errors")
            .and_then(Value::as_array)
            .is_some_and(|errors| !errors.is_empty())
}

fn empty_state() -> RawFileState {
    RawFileState {
        body_exists: false,
        metadata_exists: false,
        body_bytes: 0,
        body_sha256: None,
    }
}
