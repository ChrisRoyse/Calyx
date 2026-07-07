use std::fs;
use std::path::Path;

use serde_json::Value;

use crate::rate_limit_governor::{
    RateLimitEndpoint, RateLimitedHttpOutcome, execute_rate_limited_request, parse_retry_after_ms,
};
use crate::raw_large_corpus::{LargeCorpusEdgeCase, LargeCorpusPage, LargeCorpusRequest};
use crate::raw_large_corpus_clob_plan::{
    ClobCapturePlan, ClobTarget, clob_edge, get_plans, post_plans,
};
use crate::raw_large_corpus_profile::CorpusRecord;
use crate::raw_large_corpus_support::{collect_records, record_count};
use crate::raw_source_support::{file_state, sha256_hex, write_json};
use crate::raw_sources::RawFileState;
use crate::{PolyError, Result};

pub(crate) fn capture_clob_market_data(
    request: &LargeCorpusRequest,
    agent: &ureq::Agent,
    targets: &[ClobTarget],
    pages: &mut Vec<LargeCorpusPage>,
    records: &mut Vec<CorpusRecord>,
) -> Result<()> {
    if targets.is_empty() {
        return Err(PolyError::raw_source(
            "POLY_LARGE_CORPUS_CLOB_TARGETS_EMPTY",
            "no CLOB token IDs were derived from the persisted Gamma corpus",
        ));
    }
    for (index, target) in targets.iter().enumerate() {
        for plan in get_plans(target) {
            capture_clob_page(request, agent, &plan, index, pages, records)?;
        }
    }
    for plan in post_plans(targets) {
        capture_clob_page(request, agent, &plan, 0, pages, records)?;
    }
    Ok(())
}

pub(crate) fn capture_clob_edge_cases(
    request: &LargeCorpusRequest,
    agent: &ureq::Agent,
    targets: &[ClobTarget],
) -> Result<Vec<LargeCorpusEdgeCase>> {
    let target = targets.first().ok_or_else(|| {
        PolyError::raw_source(
            "POLY_LARGE_CORPUS_CLOB_EDGE_TARGET_MISSING",
            "no CLOB target exists for edge-case probes",
        )
    })?;
    let edges = [
        clob_edge(
            "edge_clob_book_invalid_token",
            "GET",
            "https://clob.polymarket.com/book?token_id=not-a-real-token".to_string(),
            None,
            "expected_http_failure",
        ),
        clob_edge(
            "edge_clob_price_invalid_side",
            "GET",
            format!(
                "https://clob.polymarket.com/price?token_id={}&side=HOLD",
                target.token_id
            ),
            None,
            "expected_http_failure",
        ),
        clob_edge(
            "edge_clob_prices_empty_payload",
            "POST",
            "https://clob.polymarket.com/prices".to_string(),
            Some(Value::Array(Vec::new())),
            "empty_object_success",
        ),
        clob_edge(
            "edge_clob_batch_history_missing_markets",
            "POST",
            "https://clob.polymarket.com/batch-prices-history".to_string(),
            Some(serde_json::json!({})),
            "expected_http_failure",
        ),
    ];
    edges
        .iter()
        .map(|plan| capture_clob_edge(request, agent, plan))
        .collect()
}

fn capture_clob_page(
    request: &LargeCorpusRequest,
    agent: &ureq::Agent,
    plan: &ClobCapturePlan,
    page_index: usize,
    pages: &mut Vec<LargeCorpusPage>,
    records: &mut Vec<CorpusRecord>,
) -> Result<()> {
    let dir = request.output_root.join("raw").join(plan.dataset);
    let body_path = dir.join(format!("page-{page_index:06}.json"));
    let metadata_path = dir.join(format!("page-{page_index:06}.metadata.json"));
    let request_path = dir.join(format!("page-{page_index:06}.request.json"));
    let before = file_state(&body_path, &metadata_path)?;
    fs::create_dir_all(&dir).map_err(|err| {
        PolyError::raw_source(
            "POLY_LARGE_CORPUS_CLOB_DIR_CREATE_FAILED",
            format!("create CLOB corpus dir {}: {err}", dir.display()),
        )
    })?;
    let request_bytes = persist_request_body(&request_path, &plan.request_body)?;
    let (status_code, bytes) = execute_clob_request(
        agent,
        plan.endpoint,
        plan.method,
        &plan.url,
        request_bytes.as_deref(),
        request.max_body_bytes,
    )?;
    fs::write(&body_path, &bytes).map_err(|err| {
        PolyError::raw_source(
            "POLY_LARGE_CORPUS_CLOB_BODY_WRITE_FAILED",
            format!("write CLOB body {}: {err}", body_path.display()),
        )
    })?;
    let parsed = serde_json::from_slice::<Value>(&bytes);
    let mut page = LargeCorpusPage {
        dataset: plan.dataset.to_string(),
        source: "clob".to_string(),
        endpoint: plan.endpoint.to_string(),
        method: plan.method.to_string(),
        docs_url: plan.docs_url.to_string(),
        page_index,
        url: plan.url.clone(),
        request_path: request_bytes
            .as_ref()
            .map(|_| request_path.display().to_string()),
        request_body_bytes: request_bytes.as_ref().map_or(0, Vec::len) as u64,
        request_body_sha256: request_bytes.as_ref().map(|bytes| sha256_hex(bytes)),
        status_code,
        http_success: status_code.is_some_and(|code| (200..300).contains(&code)),
        expectation_met: false,
        record_count: parsed.as_ref().map(record_count).unwrap_or(0),
        stop_reason: Some(plan.stop_reason.to_string()),
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
        after: RawFileState {
            body_exists: false,
            metadata_exists: false,
            body_bytes: 0,
            body_sha256: None,
        },
    };
    page.expectation_met = page.http_success && page.json_parse_ok && page.body_bytes > 0;
    write_json(&metadata_path, &page)?;
    page.after = file_state(&body_path, &metadata_path)?;
    if page.body_sha256 != page.after.body_sha256 {
        page.expectation_met = false;
    }
    write_json(&metadata_path, &page)?;
    collect_records(
        &page.dataset,
        &page.source,
        Path::new(&page.body_path),
        records,
    )?;
    pages.push(page);
    Ok(())
}

fn capture_clob_edge(
    request: &LargeCorpusRequest,
    agent: &ureq::Agent,
    plan: &ClobCapturePlan,
) -> Result<LargeCorpusEdgeCase> {
    let dir = request.output_root.join("edge").join(plan.dataset);
    let body_path = dir.join("body.json");
    let metadata_path = dir.join("metadata.json");
    let request_path = dir.join("request.json");
    let before = file_state(&body_path, &metadata_path)?;
    fs::create_dir_all(&dir).map_err(|err| {
        PolyError::raw_source(
            "POLY_LARGE_CORPUS_CLOB_EDGE_DIR_CREATE_FAILED",
            format!("create CLOB edge dir {}: {err}", dir.display()),
        )
    })?;
    let request_bytes = persist_request_body(&request_path, &plan.request_body)?;
    let (status_code, bytes) = execute_clob_request(
        agent,
        plan.endpoint,
        plan.method,
        &plan.url,
        request_bytes.as_deref(),
        request.max_body_bytes,
    )?;
    fs::write(&body_path, &bytes).map_err(|err| {
        PolyError::raw_source(
            "POLY_LARGE_CORPUS_CLOB_EDGE_BODY_WRITE_FAILED",
            format!("write CLOB edge body {}: {err}", body_path.display()),
        )
    })?;
    let parsed = serde_json::from_slice::<Value>(&bytes).ok();
    let http_success = status_code.is_some_and(|code| (200..300).contains(&code));
    let expectation_met = match plan.stop_reason {
        "expected_http_failure" => !http_success,
        "empty_object_success" => {
            http_success
                && parsed
                    .as_ref()
                    .and_then(Value::as_object)
                    .is_some_and(|map| map.is_empty())
        }
        _ => false,
    };
    let mut edge = LargeCorpusEdgeCase {
        name: plan.dataset.to_string(),
        method: plan.method.to_string(),
        url: plan.url.clone(),
        request_path: request_bytes
            .as_ref()
            .map(|_| request_path.display().to_string()),
        request_body_bytes: request_bytes.as_ref().map_or(0, Vec::len) as u64,
        request_body_sha256: request_bytes.as_ref().map(|bytes| sha256_hex(bytes)),
        expected_semantics: plan.stop_reason.to_string(),
        status_code,
        expectation_met,
        record_count: parsed.as_ref().map(record_count).unwrap_or(0),
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
        after: RawFileState {
            body_exists: false,
            metadata_exists: false,
            body_bytes: 0,
            body_sha256: None,
        },
    };
    write_json(&metadata_path, &edge)?;
    edge.after = file_state(&body_path, &metadata_path)?;
    if edge.body_sha256 != edge.after.body_sha256 {
        edge.expectation_met = false;
    }
    write_json(&metadata_path, &edge)?;
    Ok(edge)
}

fn persist_request_body(path: &Path, body: &Option<Value>) -> Result<Option<Vec<u8>>> {
    let Some(body) = body else {
        return Ok(None);
    };
    let bytes = serde_json::to_vec(body).map_err(|err| {
        PolyError::raw_source(
            "POLY_LARGE_CORPUS_CLOB_REQUEST_ENCODE_FAILED",
            format!("encode CLOB request body {}: {err}", path.display()),
        )
    })?;
    fs::write(path, &bytes).map_err(|err| {
        PolyError::raw_source(
            "POLY_LARGE_CORPUS_CLOB_REQUEST_WRITE_FAILED",
            format!("write CLOB request body {}: {err}", path.display()),
        )
    })?;
    let readback = fs::read(path).map_err(|err| {
        PolyError::raw_source(
            "POLY_LARGE_CORPUS_CLOB_REQUEST_READBACK_FAILED",
            format!("read back CLOB request body {}: {err}", path.display()),
        )
    })?;
    if readback != bytes {
        return Err(PolyError::raw_source(
            "POLY_LARGE_CORPUS_CLOB_REQUEST_READBACK_MISMATCH",
            format!(
                "CLOB request body readback mismatch at {}; expected_sha256={} actual_sha256={}",
                path.display(),
                sha256_hex(&bytes),
                sha256_hex(&readback)
            ),
        ));
    }
    Ok(Some(bytes))
}

fn execute_clob_request(
    agent: &ureq::Agent,
    endpoint_name: &str,
    method: &str,
    url: &str,
    request_body: Option<&[u8]>,
    limit: usize,
) -> Result<(Option<u16>, Vec<u8>)> {
    let endpoint = RateLimitEndpoint::new("clob", endpoint_name, method);
    execute_rate_limited_request(&endpoint, || {
        let result = match method {
            "GET" => agent.get(url).header("Accept", "application/json").call(),
            "POST" => {
                let body = request_body.ok_or_else(|| {
                    PolyError::raw_source(
                        "POLY_LARGE_CORPUS_CLOB_POST_BODY_MISSING",
                        format!("POST CLOB request {url} has no persisted body"),
                    )
                })?;
                agent
                    .post(url)
                    .header("Accept", "application/json")
                    .header("Content-Type", "application/json")
                    .send(body)
            }
            other => {
                return Err(PolyError::raw_source(
                    "POLY_LARGE_CORPUS_CLOB_METHOD_UNSUPPORTED",
                    format!("unsupported CLOB method {other} for {url}"),
                ));
            }
        };
        let mut response = result.map_err(|err| {
            PolyError::raw_source(
                "POLY_LARGE_CORPUS_CLOB_TRANSPORT_FAILED",
                format!("fetch CLOB {method} {url}: {err}"),
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
                    "POLY_LARGE_CORPUS_CLOB_BODY_READ_FAILED",
                    format!("read CLOB body from {url}: {err}"),
                )
            })?;
        Ok(RateLimitedHttpOutcome {
            status_code,
            retry_after_ms,
            value: (status_code, bytes),
        })
    })
}
