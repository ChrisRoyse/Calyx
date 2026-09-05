use std::fs;
use std::path::Path;

use calyx_core::Clock;
use serde_json::{Value, json};

use crate::raw_large_corpus::{
    LargeCorpusEdgeCase, LargeCorpusPage, LargeCorpusRangeState, LargeCorpusRequest,
};
pub(crate) use crate::raw_large_corpus_onchain_chunks_http::execute_post;
use crate::raw_large_corpus_profile::CorpusRecord;
use crate::raw_large_corpus_support::{record_count, records_from_value};
use crate::raw_source_support::{file_state, sha256_hex, write_json};
use crate::raw_sources::RawFileState;
use crate::{PolyError, Result};

pub(crate) const POLYGON_RPC_SAFE_LOG_CHUNK_BLOCKS: u64 = 10;

const POLYGON_RPC_SAMPLE_CHUNK_LIMIT: usize = 5;
const ZERO_ADDRESS: &str = "0x0000000000000000000000000000000000000000";
const INVALID_TOPIC: &str = "0x1234";

pub(crate) fn capture_polygon_chunked_log_pages(
    request: &LargeCorpusRequest,
    agent: &ureq::Agent,
    latest_block: u64,
    spec: ChunkSourceSpec<'_>,
    clock: &dyn Clock,
) -> Result<Vec<LargeCorpusPage>> {
    let windows = recent_windows(
        latest_block,
        request
            .max_pages_per_dataset
            .min(POLYGON_RPC_SAMPLE_CHUNK_LIMIT),
        POLYGON_RPC_SAFE_LOG_CHUNK_BLOCKS,
    );
    let chunk_count = windows.len();
    let mut pages = Vec::new();
    for (chunk_index, (from_block, to_block)) in windows.into_iter().enumerate() {
        let plan = ChunkPlan::within_limit(spec, from_block, to_block, chunk_index, chunk_count);
        pages.push(capture_chunk_page(request, agent, &spec, &plan, clock)?);
    }
    Ok(pages)
}

pub(crate) fn collect_polygon_chunk_records(
    page: &LargeCorpusPage,
    records: &mut Vec<CorpusRecord>,
) -> Result<()> {
    let bytes = fs::read(Path::new(&page.body_path)).map_err(|err| {
        PolyError::raw_source(
            "POLY_LARGE_CORPUS_ONCHAIN_CHUNK_RECORD_READ_FAILED",
            format!("read chunk page {}: {err}", page.body_path),
        )
    })?;
    let value = serde_json::from_slice::<Value>(&bytes).map_err(|err| {
        PolyError::raw_source(
            "POLY_LARGE_CORPUS_ONCHAIN_CHUNK_RECORD_DECODE_FAILED",
            format!("decode chunk page {}: {err}", page.body_path),
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

pub(crate) fn capture_polygon_chunk_edge_cases(
    request: &LargeCorpusRequest,
    agent: &ureq::Agent,
    latest_block: u64,
    spec: ChunkSourceSpec<'_>,
    clock: &dyn Clock,
) -> Result<Vec<LargeCorpusEdgeCase>> {
    let safe_from = latest_block.saturating_sub(POLYGON_RPC_SAFE_LOG_CHUNK_BLOCKS - 1);
    let plans = [
        ChunkPlan::edge(
            "edge_polygon_rpc_chunk_safe_range_large",
            "safe-10-block-range",
            spec.address,
            spec.topic,
            (safe_from, latest_block),
            "expected_chunk_success",
            "within_chunk_limit",
        ),
        ChunkPlan::edge(
            "edge_polygon_rpc_empty_zero_address_range_large",
            "empty-zero-address-range",
            ZERO_ADDRESS,
            spec.topic,
            (safe_from, latest_block),
            "expected_empty_result_success",
            "within_chunk_limit",
        ),
        ChunkPlan::edge(
            "edge_polygon_rpc_too_large_range_large",
            "too-large-10001-block-range",
            spec.address,
            spec.topic,
            (latest_block.saturating_sub(10_000), latest_block),
            "expected_json_rpc_range_error",
            "expected_over_limit",
        ),
        ChunkPlan::edge(
            "edge_polygon_rpc_invalid_topic_large",
            "invalid-topic",
            spec.address,
            INVALID_TOPIC,
            (latest_block, latest_block),
            "expected_json_rpc_error",
            "within_chunk_limit",
        ),
    ];
    plans
        .iter()
        .map(|plan| capture_chunk_edge(request, agent, &spec, plan, clock))
        .collect()
}

pub(crate) fn capture_chunk_page(
    request: &LargeCorpusRequest,
    agent: &ureq::Agent,
    spec: &ChunkSourceSpec<'_>,
    plan: &ChunkPlan,
    clock: &dyn Clock,
) -> Result<LargeCorpusPage> {
    let dir = request.output_root.join("raw").join(&plan.dataset);
    let page_name = format!("page-{:06}", plan.chunk_index);
    let body_path = dir.join(format!("{page_name}.json"));
    let metadata_path = dir.join(format!("{page_name}.metadata.json"));
    let request_path = dir.join(format!("{page_name}.request.json"));
    let before = file_state(&body_path, &metadata_path)?;
    fs::create_dir_all(&dir).map_err(|err| {
        PolyError::raw_source(
            "POLY_LARGE_CORPUS_ONCHAIN_CHUNK_DIR_CREATE_FAILED",
            format!("create chunk dir {}: {err}", dir.display()),
        )
    })?;
    let request_body = chunk_request(plan);
    let request_bytes = persist_request(&request_path, &request_body)?;
    let (status_code, bytes, transport_error) = execute_post(
        clock,
        agent,
        spec.rpc_url,
        &request_bytes,
        request.max_body_bytes,
        &plan.dataset,
    )?;
    fs::write(&body_path, &bytes).map_err(|err| {
        PolyError::raw_source(
            "POLY_LARGE_CORPUS_ONCHAIN_CHUNK_BODY_WRITE_FAILED",
            format!("write chunk body {}: {err}", body_path.display()),
        )
    })?;
    let parsed = serde_json::from_slice::<Value>(&bytes);
    let http_success = status_code.is_some_and(|code| (200..300).contains(&code));
    let has_error = parsed.as_ref().is_ok_and(has_response_error);
    let mut page = LargeCorpusPage {
        dataset: plan.dataset.clone(),
        source: "polygon-rpc".to_string(),
        endpoint: plan.endpoint.clone(),
        method: "POST".to_string(),
        docs_url: spec.docs_url.to_string(),
        page_index: plan.chunk_index,
        url: spec.rpc_url.to_string(),
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
        stop_reason: Some("chunked_json_rpc_logs".to_string()),
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
        range_state: Some(range_state(plan)),
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

fn capture_chunk_edge(
    request: &LargeCorpusRequest,
    agent: &ureq::Agent,
    spec: &ChunkSourceSpec<'_>,
    plan: &ChunkPlan,
    clock: &dyn Clock,
) -> Result<LargeCorpusEdgeCase> {
    let dir = request.output_root.join("edge").join(&plan.dataset);
    let body_path = dir.join("body.json");
    let metadata_path = dir.join("metadata.json");
    let request_path = dir.join("request.json");
    let before = file_state(&body_path, &metadata_path)?;
    fs::create_dir_all(&dir).map_err(|err| {
        PolyError::raw_source(
            "POLY_LARGE_CORPUS_ONCHAIN_CHUNK_EDGE_DIR_CREATE_FAILED",
            format!("create chunk edge dir {}: {err}", dir.display()),
        )
    })?;
    let request_body = chunk_request(plan);
    let request_bytes = persist_request(&request_path, &request_body)?;
    let (status_code, bytes, transport_error) = execute_post(
        clock,
        agent,
        spec.rpc_url,
        &request_bytes,
        request.max_body_bytes,
        &plan.dataset,
    )?;
    fs::write(&body_path, &bytes).map_err(|err| {
        PolyError::raw_source(
            "POLY_LARGE_CORPUS_ONCHAIN_CHUNK_EDGE_BODY_WRITE_FAILED",
            format!("write chunk edge body {}: {err}", body_path.display()),
        )
    })?;
    let parsed = serde_json::from_slice::<Value>(&bytes);
    let http_success = status_code.is_some_and(|code| (200..300).contains(&code));
    let has_error = parsed.as_ref().is_ok_and(has_response_error);
    let expectation_met = match plan.expected_semantics.as_str() {
        "expected_chunk_success" => {
            http_success && parsed.is_ok() && !has_error && transport_error.is_none()
        }
        "expected_empty_result_success" => {
            http_success && parsed.as_ref().is_ok_and(result_is_empty_array)
        }
        "expected_json_rpc_error" | "expected_json_rpc_range_error" => {
            parsed.is_ok() && has_error && transport_error.is_none()
        }
        _ => false,
    };
    let mut edge = LargeCorpusEdgeCase {
        name: plan.dataset.clone(),
        method: "POST".to_string(),
        url: spec.rpc_url.to_string(),
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
        range_state: Some(range_state(plan)),
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

fn recent_windows(latest_block: u64, chunk_count: usize, chunk_size: u64) -> Vec<(u64, u64)> {
    let total_blocks = chunk_size.saturating_mul(chunk_count as u64);
    let mut from_block = latest_block.saturating_sub(total_blocks.saturating_sub(1));
    let mut windows = Vec::new();
    while from_block <= latest_block && windows.len() < chunk_count {
        let to_block = from_block
            .saturating_add(chunk_size.saturating_sub(1))
            .min(latest_block);
        windows.push((from_block, to_block));
        if to_block == latest_block {
            break;
        }
        from_block = to_block.saturating_add(1);
    }
    windows
}

fn chunk_request(plan: &ChunkPlan) -> Value {
    json_rpc(
        "eth_getLogs",
        json!([{
            "fromBlock": hex_block(plan.from_block),
            "toBlock": hex_block(plan.to_block),
            "address": plan.address,
            "topics": [plan.topic]
        }]),
    )
}

fn range_state(plan: &ChunkPlan) -> LargeCorpusRangeState {
    let requested_block_count = plan.to_block - plan.from_block + 1;
    let next_from_block = (plan.chunk_index + 1 < plan.chunk_count).then_some(plan.to_block + 1);
    LargeCorpusRangeState {
        chain: "polygon".to_string(),
        address: plan.address.clone(),
        topics: vec![plan.topic.clone()],
        from_block: plan.from_block,
        to_block: plan.to_block,
        requested_block_count,
        max_blocks_per_chunk: requested_block_count,
        chunk_index: plan.chunk_index,
        chunk_count: plan.chunk_count,
        next_from_block,
        range_policy: "single_contract_topic_filtered_block_window".to_string(),
        limit_semantics: plan.limit_semantics.clone(),
        provider_limit_evidence: "live_drpc_range_threshold_probe_and_provider_docs".to_string(),
    }
}

fn persist_request(path: &Path, body: &Value) -> Result<Vec<u8>> {
    let bytes = serde_json::to_vec_pretty(body).map_err(|err| {
        PolyError::raw_source(
            "POLY_LARGE_CORPUS_ONCHAIN_CHUNK_REQUEST_ENCODE_FAILED",
            format!("encode chunk request {}: {err}", path.display()),
        )
    })?;
    fs::write(path, &bytes).map_err(|err| {
        PolyError::raw_source(
            "POLY_LARGE_CORPUS_ONCHAIN_CHUNK_REQUEST_WRITE_FAILED",
            format!("write chunk request {}: {err}", path.display()),
        )
    })?;
    let readback = fs::read(path).map_err(|err| {
        PolyError::raw_source(
            "POLY_LARGE_CORPUS_ONCHAIN_CHUNK_REQUEST_READBACK_FAILED",
            format!("read chunk request {}: {err}", path.display()),
        )
    })?;
    if readback != bytes {
        return Err(PolyError::raw_source(
            "POLY_LARGE_CORPUS_ONCHAIN_CHUNK_REQUEST_READBACK_MISMATCH",
            format!("chunk request readback mismatch at {}", path.display()),
        ));
    }
    Ok(bytes)
}

fn has_response_error(value: &Value) -> bool {
    value.get("error").is_some()
}

fn result_is_empty_array(value: &Value) -> bool {
    value
        .get("result")
        .and_then(Value::as_array)
        .is_some_and(Vec::is_empty)
}

fn json_rpc(method: &str, params: Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": method,
        "params": params
    })
}

fn hex_block(block: u64) -> String {
    format!("0x{block:x}")
}

fn empty_state() -> RawFileState {
    RawFileState {
        body_exists: false,
        metadata_exists: false,
        body_bytes: 0,
        body_sha256: None,
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ChunkSourceSpec<'a> {
    pub(crate) dataset: &'a str,
    pub(crate) endpoint: &'a str,
    pub(crate) address: &'a str,
    pub(crate) topic: &'a str,
    pub(crate) rpc_url: &'a str,
    pub(crate) docs_url: &'a str,
}

#[derive(Debug, Clone)]
pub(crate) struct ChunkPlan {
    dataset: String,
    endpoint: String,
    address: String,
    topic: String,
    from_block: u64,
    to_block: u64,
    chunk_index: usize,
    chunk_count: usize,
    expected_semantics: String,
    limit_semantics: String,
}

impl ChunkPlan {
    pub(crate) fn within_limit(
        spec: ChunkSourceSpec<'_>,
        from_block: u64,
        to_block: u64,
        chunk_index: usize,
        chunk_count: usize,
    ) -> Self {
        Self {
            dataset: spec.dataset.to_string(),
            endpoint: spec.endpoint.to_string(),
            address: spec.address.to_string(),
            topic: spec.topic.to_string(),
            from_block,
            to_block,
            chunk_index,
            chunk_count,
            expected_semantics: "expected_chunk_success".to_string(),
            limit_semantics: "within_chunk_limit".to_string(),
        }
    }

    fn edge(
        dataset: &str,
        endpoint: &str,
        address: &str,
        topic: &str,
        blocks: (u64, u64),
        expected_semantics: &str,
        limit_semantics: &str,
    ) -> Self {
        Self {
            dataset: dataset.to_string(),
            endpoint: endpoint.to_string(),
            address: address.to_string(),
            topic: topic.to_string(),
            from_block: blocks.0,
            to_block: blocks.1,
            chunk_index: 0,
            chunk_count: 1,
            expected_semantics: expected_semantics.to_string(),
            limit_semantics: limit_semantics.to_string(),
        }
    }
}
