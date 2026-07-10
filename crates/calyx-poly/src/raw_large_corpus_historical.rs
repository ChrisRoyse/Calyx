use std::fs;
use std::path::Path;

use calyx_core::Clock;
use serde_json::{Value, json};

use crate::raw_historical_support::{HistoricalFormat, execute_get, validate_body};
use crate::raw_large_corpus::{LargeCorpusEdgeCase, LargeCorpusPage, LargeCorpusRequest};
use crate::raw_large_corpus_profile::CorpusRecord;
use crate::raw_large_corpus_support::records_from_value;
use crate::raw_source_support::{file_state, sha256_hex, write_json};
use crate::raw_sources::RawFileState;
use crate::{PolyError, Result};

const SOURCE: &str = "historical-dump";
const HF_SIMPLE_DOCS: &str = "https://huggingface.co/datasets/SimpleFunctions/settled-markets";
const HF_COGNOCRACY_DOCS: &str =
    "https://huggingface.co/datasets/cognocracy-agent/polymarket-gamma-dataset";
const HF_TIMESEVENTEEN_DOCS: &str = "https://huggingface.co/datasets/TimeSeventeen/Polymarket-v1";

pub(crate) fn capture_historical_market_data(
    request: &LargeCorpusRequest,
    agent: &ureq::Agent,
    pages: &mut Vec<LargeCorpusPage>,
    records: &mut Vec<CorpusRecord>,
    clock: &dyn Clock,
) -> Result<()> {
    for plan in historical_plans() {
        let page = capture_historical_page(request, agent, &plan, clock)?;
        collect_historical_records(&page, &plan, records)?;
        pages.push(page);
    }
    Ok(())
}

pub(crate) fn capture_historical_edge_cases(
    request: &LargeCorpusRequest,
    agent: &ureq::Agent,
    clock: &dyn Clock,
) -> Result<Vec<LargeCorpusEdgeCase>> {
    historical_edge_plans()
        .iter()
        .map(|plan| capture_historical_edge(request, agent, plan, clock))
        .collect()
}

fn historical_plans() -> Vec<HistoricalPlan> {
    vec![
        plan(
            "historical_hf_simplefunctions_tree_large",
            "settled-markets-tree",
            "https://huggingface.co/api/datasets/SimpleFunctions/settled-markets/tree/main?recursive=true",
            HF_SIMPLE_DOCS,
            HistoricalFormat::HfTreeJson,
            "sampled_hf_tree",
        ),
        plan(
            "historical_hf_simplefunctions_2026_04_jsonl_large",
            "settled-markets-monthly-jsonl",
            "https://huggingface.co/datasets/SimpleFunctions/settled-markets/resolve/main/2026-04.jsonl",
            HF_SIMPLE_DOCS,
            HistoricalFormat::Jsonl,
            "sampled_jsonl_partition",
        ),
        plan(
            "historical_hf_cognocracy_manifest_large",
            "gamma-dataset-manifest",
            "https://huggingface.co/datasets/cognocracy-agent/polymarket-gamma-dataset/resolve/main/manifest.json",
            HF_COGNOCRACY_DOCS,
            HistoricalFormat::Json,
            "sampled_manifest",
        ),
        plan(
            "historical_hf_timeseventeen_tree_large",
            "polymarket-v1-tree",
            "https://huggingface.co/api/datasets/TimeSeventeen/Polymarket-v1/tree/main?recursive=true",
            HF_TIMESEVENTEEN_DOCS,
            HistoricalFormat::HfTreeJson,
            "sampled_hf_tree",
        ),
        plan(
            "historical_hf_timeseventeen_readme_large",
            "polymarket-v1-readme",
            "https://huggingface.co/datasets/TimeSeventeen/Polymarket-v1/resolve/main/README.md",
            HF_TIMESEVENTEEN_DOCS,
            HistoricalFormat::Text,
            "sampled_readme_text",
        ),
        plan(
            "historical_hf_timeseventeen_daily_aligned_parquet_large",
            "polymarket-v1-daily-aligned-parquet",
            "https://huggingface.co/datasets/TimeSeventeen/Polymarket-v1/resolve/main/daily_aligned/2022-11-21.parquet",
            HF_TIMESEVENTEEN_DOCS,
            HistoricalFormat::Binary,
            "sampled_binary_partition",
        ),
    ]
}

fn historical_edge_plans() -> Vec<HistoricalPlan> {
    vec![
        plan(
            "edge_historical_hf_simplefunctions_missing_month",
            "missing-settled-markets-month",
            "https://huggingface.co/datasets/SimpleFunctions/settled-markets/resolve/main/1900-01.jsonl",
            HF_SIMPLE_DOCS,
            HistoricalFormat::Text,
            "expected_http_failure",
        ),
        plan(
            "edge_historical_hf_missing_dataset_tree",
            "missing-huggingface-dataset-tree",
            "https://huggingface.co/api/datasets/PolyDefinitelyMissingDatasetForFsv/tree/main?recursive=true",
            "https://huggingface.co/docs/hub/api",
            HistoricalFormat::Text,
            "expected_http_failure",
        ),
        plan(
            "edge_historical_hf_readme_as_jsonl_invalid",
            "readme-as-jsonl-invalid",
            "https://huggingface.co/datasets/SimpleFunctions/settled-markets/resolve/main/README.md",
            HF_SIMPLE_DOCS,
            HistoricalFormat::Jsonl,
            "expected_format_failure",
        ),
    ]
}

fn capture_historical_page(
    request: &LargeCorpusRequest,
    agent: &ureq::Agent,
    plan: &HistoricalPlan,
    clock: &dyn Clock,
) -> Result<LargeCorpusPage> {
    let dir = request.output_root.join("raw").join(&plan.dataset);
    let body_path = dir.join(format!("page-000000.{}", body_extension(plan.format)));
    let metadata_path = dir.join("page-000000.metadata.json");
    let before = file_state(&body_path, &metadata_path)?;
    fs::create_dir_all(&dir).map_err(|err| {
        PolyError::raw_source(
            "POLY_LARGE_CORPUS_HISTORICAL_DIR_CREATE_FAILED",
            format!("create historical corpus dir {}: {err}", dir.display()),
        )
    })?;
    let (status_code, bytes, transport_error) =
        execute_historical_get(agent, request, plan, clock)?;
    fs::write(&body_path, &bytes).map_err(|err| {
        PolyError::raw_source(
            "POLY_LARGE_CORPUS_HISTORICAL_BODY_WRITE_FAILED",
            format!(
                "write historical corpus body {}: {err}",
                body_path.display()
            ),
        )
    })?;
    let validation = validate_body(plan.format, &bytes);
    let http_success = status_code.is_some_and(|code| (200..300).contains(&code));
    let mut page = LargeCorpusPage {
        dataset: plan.dataset.clone(),
        source: SOURCE.to_string(),
        endpoint: plan.endpoint.clone(),
        method: "GET".to_string(),
        docs_url: plan.docs_url.clone(),
        page_index: 0,
        url: plan.url.clone(),
        request_path: None,
        request_body_bytes: 0,
        request_body_sha256: None,
        status_code,
        http_success,
        expectation_met: http_success && validation.ok && transport_error.is_none(),
        record_count: validation.record_count.unwrap_or(0),
        stop_reason: Some(plan.expected_semantics.clone()),
        body_path: body_path.display().to_string(),
        metadata_path: metadata_path.display().to_string(),
        body_format: format_name(plan.format).to_string(),
        body_bytes: bytes.len() as u64,
        body_sha256: Some(sha256_hex(&bytes)),
        json_parse_ok: validation.ok,
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

fn capture_historical_edge(
    request: &LargeCorpusRequest,
    agent: &ureq::Agent,
    plan: &HistoricalPlan,
    clock: &dyn Clock,
) -> Result<LargeCorpusEdgeCase> {
    let dir = request.output_root.join("edge").join(&plan.dataset);
    let body_path = dir.join(format!("body.{}", body_extension(plan.format)));
    let metadata_path = dir.join("metadata.json");
    let before = file_state(&body_path, &metadata_path)?;
    fs::create_dir_all(&dir).map_err(|err| {
        PolyError::raw_source(
            "POLY_LARGE_CORPUS_HISTORICAL_EDGE_DIR_CREATE_FAILED",
            format!("create historical edge dir {}: {err}", dir.display()),
        )
    })?;
    let (status_code, bytes, transport_error) =
        execute_historical_get(agent, request, plan, clock)?;
    fs::write(&body_path, &bytes).map_err(|err| {
        PolyError::raw_source(
            "POLY_LARGE_CORPUS_HISTORICAL_EDGE_BODY_WRITE_FAILED",
            format!("write historical edge body {}: {err}", body_path.display()),
        )
    })?;
    let validation = validate_body(plan.format, &bytes);
    let http_success = status_code.is_some_and(|code| (200..300).contains(&code));
    let expectation_met = match plan.expected_semantics.as_str() {
        "expected_http_failure" => !http_success || transport_error.is_some(),
        "expected_format_failure" => http_success && !validation.ok,
        _ => false,
    };
    let mut edge = LargeCorpusEdgeCase {
        name: plan.dataset.clone(),
        method: "GET".to_string(),
        url: plan.url.clone(),
        request_path: None,
        request_body_bytes: 0,
        request_body_sha256: None,
        expected_semantics: plan.expected_semantics.clone(),
        status_code,
        expectation_met,
        record_count: validation.record_count.unwrap_or(0),
        body_path: body_path.display().to_string(),
        metadata_path: metadata_path.display().to_string(),
        body_format: format_name(plan.format).to_string(),
        json_parse_ok: validation.ok,
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

fn execute_historical_get(
    agent: &ureq::Agent,
    request: &LargeCorpusRequest,
    plan: &HistoricalPlan,
    clock: &dyn Clock,
) -> Result<(Option<u16>, Vec<u8>, Option<String>)> {
    execute_get(
        clock,
        agent,
        &plan.dataset,
        &plan.url,
        plan.format,
        request.max_body_bytes as u64,
    )
}

fn collect_historical_records(
    page: &LargeCorpusPage,
    plan: &HistoricalPlan,
    records: &mut Vec<CorpusRecord>,
) -> Result<()> {
    let bytes = fs::read(Path::new(&page.body_path)).map_err(|err| {
        PolyError::raw_source(
            "POLY_LARGE_CORPUS_HISTORICAL_RECORD_READ_FAILED",
            format!("read historical page {}: {err}", page.body_path),
        )
    })?;
    let values = match plan.format {
        HistoricalFormat::Json | HistoricalFormat::HfTreeJson => {
            let value = serde_json::from_slice::<Value>(&bytes).map_err(|err| {
                PolyError::raw_source(
                    "POLY_LARGE_CORPUS_HISTORICAL_JSON_DECODE_FAILED",
                    format!("decode historical JSON {}: {err}", page.body_path),
                )
            })?;
            records_from_value(&value)
        }
        HistoricalFormat::Jsonl => jsonl_records(&bytes, &page.body_path)?,
        HistoricalFormat::Text | HistoricalFormat::Binary => vec![json!({
            "dataset": page.dataset,
            "source_format": page.body_format,
            "body_bytes": page.body_bytes,
            "body_sha256": page.body_sha256,
            "docs_url": page.docs_url,
        })],
    };
    for value in values {
        records.push(CorpusRecord {
            dataset: page.dataset.clone(),
            source: page.source.clone(),
            value,
        });
    }
    Ok(())
}

fn jsonl_records(bytes: &[u8], path: &str) -> Result<Vec<Value>> {
    let text = std::str::from_utf8(bytes).map_err(|err| {
        PolyError::raw_source(
            "POLY_LARGE_CORPUS_HISTORICAL_JSONL_UTF8_FAILED",
            format!("decode JSONL UTF-8 {path}: {err}"),
        )
    })?;
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            serde_json::from_str::<Value>(line).map_err(|err| {
                PolyError::raw_source(
                    "POLY_LARGE_CORPUS_HISTORICAL_JSONL_DECODE_FAILED",
                    format!("decode JSONL record {path}: {err}"),
                )
            })
        })
        .collect()
}

fn plan(
    dataset: &str,
    endpoint: &str,
    url: &str,
    docs_url: &str,
    format: HistoricalFormat,
    expected_semantics: &str,
) -> HistoricalPlan {
    HistoricalPlan {
        dataset: dataset.to_string(),
        endpoint: endpoint.to_string(),
        url: url.to_string(),
        docs_url: docs_url.to_string(),
        format,
        expected_semantics: expected_semantics.to_string(),
    }
}

fn body_extension(format: HistoricalFormat) -> &'static str {
    match format {
        HistoricalFormat::Json | HistoricalFormat::HfTreeJson => "json",
        HistoricalFormat::Jsonl => "jsonl",
        HistoricalFormat::Text => "txt",
        HistoricalFormat::Binary => "bin",
    }
}

fn format_name(format: HistoricalFormat) -> &'static str {
    match format {
        HistoricalFormat::Json | HistoricalFormat::HfTreeJson => "json",
        HistoricalFormat::Jsonl => "jsonl",
        HistoricalFormat::Text => "text",
        HistoricalFormat::Binary => "binary",
    }
}

fn empty_state() -> RawFileState {
    RawFileState {
        body_exists: false,
        metadata_exists: false,
        body_bytes: 0,
        body_sha256: None,
    }
}

#[derive(Debug, Clone)]
struct HistoricalPlan {
    dataset: String,
    endpoint: String,
    url: String,
    docs_url: String,
    format: HistoricalFormat,
    expected_semantics: String,
}
