use std::fs;

use crate::raw_historical_support::{
    HistoricalFormat, HistoricalValidation, execute_get, persist_body, remove_stale_request,
    validate_body,
};
use crate::raw_source_support::{file_state, sanitize_segment, sha256_hex, write_json};
use crate::raw_sources::{RawEndpointSample, RawFileState, RawSourceSamplingRequest};
use crate::{PolyError, Result};

const SOURCE: &str = "historical-dump";
const HF_SIMPLE_FUNCTIONS_DOCS: &str =
    "https://huggingface.co/datasets/SimpleFunctions/settled-markets";
const HF_COGNOCRACY_DOCS: &str =
    "https://huggingface.co/datasets/cognocracy-agent/polymarket-gamma-dataset";
const HF_TIMESEVENTEEN_DOCS: &str = "https://huggingface.co/datasets/TimeSeventeen/Polymarket-v1";

pub(crate) fn capture_historical_samples(
    request: &RawSourceSamplingRequest,
    agent: &ureq::Agent,
) -> Result<Vec<RawEndpointSample>> {
    historical_probes()
        .iter()
        .map(|probe| capture_historical_probe(request, agent, probe))
        .collect()
}

fn historical_probes() -> Vec<HistoricalProbe> {
    vec![
        probe(
            "hf_simplefunctions_settled_markets_tree",
            "settled-markets-tree",
            "https://huggingface.co/api/datasets/SimpleFunctions/settled-markets/tree/main?recursive=true",
            HF_SIMPLE_FUNCTIONS_DOCS,
            HistoricalFormat::HfTreeJson,
            true,
            false,
        ),
        probe(
            "hf_simplefunctions_settled_markets_readme",
            "settled-markets-readme",
            "https://huggingface.co/datasets/SimpleFunctions/settled-markets/resolve/main/README.md",
            HF_SIMPLE_FUNCTIONS_DOCS,
            HistoricalFormat::Text,
            true,
            false,
        ),
        probe(
            "hf_simplefunctions_settled_markets_2026_04_jsonl",
            "settled-markets-monthly-jsonl",
            "https://huggingface.co/datasets/SimpleFunctions/settled-markets/resolve/main/2026-04.jsonl",
            HF_SIMPLE_FUNCTIONS_DOCS,
            HistoricalFormat::Jsonl,
            true,
            false,
        ),
        probe(
            "hf_cognocracy_gamma_manifest",
            "gamma-dataset-manifest",
            "https://huggingface.co/datasets/cognocracy-agent/polymarket-gamma-dataset/resolve/main/manifest.json",
            HF_COGNOCRACY_DOCS,
            HistoricalFormat::Json,
            true,
            false,
        ),
        probe(
            "hf_cognocracy_gamma_final_report",
            "gamma-dataset-final-report",
            "https://huggingface.co/datasets/cognocracy-agent/polymarket-gamma-dataset/resolve/main/FINAL_REPORT.txt",
            HF_COGNOCRACY_DOCS,
            HistoricalFormat::Text,
            true,
            false,
        ),
        probe(
            "hf_timeseventeen_polymarket_v1_tree",
            "polymarket-v1-tree",
            "https://huggingface.co/api/datasets/TimeSeventeen/Polymarket-v1/tree/main?recursive=true",
            HF_TIMESEVENTEEN_DOCS,
            HistoricalFormat::HfTreeJson,
            true,
            false,
        ),
        probe(
            "hf_timeseventeen_polymarket_v1_readme",
            "polymarket-v1-readme",
            "https://huggingface.co/datasets/TimeSeventeen/Polymarket-v1/resolve/main/README.md",
            HF_TIMESEVENTEEN_DOCS,
            HistoricalFormat::Text,
            true,
            false,
        ),
        probe(
            "hf_timeseventeen_daily_aligned_sample_parquet",
            "polymarket-v1-daily-aligned-parquet",
            "https://huggingface.co/datasets/TimeSeventeen/Polymarket-v1/resolve/main/daily_aligned/2022-11-21.parquet",
            HF_TIMESEVENTEEN_DOCS,
            HistoricalFormat::Binary,
            true,
            false,
        ),
        probe(
            "edge_hf_simplefunctions_missing_month",
            "missing-settled-markets-month",
            "https://huggingface.co/datasets/SimpleFunctions/settled-markets/resolve/main/1900-01.jsonl",
            HF_SIMPLE_FUNCTIONS_DOCS,
            HistoricalFormat::Jsonl,
            false,
            true,
        ),
        probe(
            "edge_hf_missing_dataset_tree",
            "missing-huggingface-dataset-tree",
            "https://huggingface.co/api/datasets/PolyDefinitelyMissingDatasetForFsv/tree/main?recursive=true",
            "https://huggingface.co/docs/hub/api",
            HistoricalFormat::HfTreeJson,
            false,
            true,
        ),
        probe(
            "edge_hf_readme_as_jsonl_invalid",
            "readme-as-jsonl-invalid",
            "https://huggingface.co/datasets/SimpleFunctions/settled-markets/resolve/main/README.md",
            HF_SIMPLE_FUNCTIONS_DOCS,
            HistoricalFormat::Jsonl,
            false,
            true,
        ),
    ]
}

fn capture_historical_probe(
    request: &RawSourceSamplingRequest,
    agent: &ureq::Agent,
    probe: &HistoricalProbe,
) -> Result<RawEndpointSample> {
    let sample_dir = request
        .output_root
        .join("raw")
        .join(SOURCE)
        .join(sanitize_segment(&probe.name));
    let body_path = sample_dir.join(probe.format.body_file());
    let metadata_path = sample_dir.join("metadata.json");
    let before = file_state(&body_path, &metadata_path)?;
    fs::create_dir_all(&sample_dir).map_err(|err| {
        PolyError::raw_source(
            "POLY_RAW_SOURCE_HISTORICAL_DIR_CREATE_FAILED",
            format!(
                "create historical sample directory {}: {err}",
                sample_dir.display()
            ),
        )
    })?;
    remove_stale_request(&sample_dir)?;
    let max_body_bytes = u64::try_from(request.max_body_bytes).map_err(|err| {
        PolyError::raw_source(
            "POLY_RAW_SOURCE_HISTORICAL_BODY_LIMIT_CONVERT_FAILED",
            format!(
                "convert max body bytes {} to u64: {err}",
                request.max_body_bytes
            ),
        )
    })?;
    let (status_code, bytes, transport_error) =
        execute_get(agent, &probe.name, &probe.url, probe.format, max_body_bytes)?;
    persist_body(&body_path, &bytes)?;

    let validation = validate_body(probe.format, &bytes);
    let http_success = status_code.is_some_and(|code| (200..300).contains(&code));
    let expectation_met = if probe.expected_success {
        http_success && validation.ok && transport_error.is_none()
    } else {
        !http_success || !validation.ok || transport_error.is_some()
    };
    let body_sha256 = (!bytes.is_empty()).then(|| sha256_hex(&bytes));
    let mut sample = RawEndpointSample {
        name: probe.name.clone(),
        source: SOURCE.to_string(),
        transport: "http".to_string(),
        endpoint: probe.endpoint.clone(),
        method: "GET".to_string(),
        url: probe.url.clone(),
        docs_url: probe.docs_url.clone(),
        request_body_exists: false,
        request_body_bytes: 0,
        request_body_sha256: None,
        request_body_path: None,
        expected_success: probe.expected_success,
        edge_case: probe.edge_case,
        status_code,
        http_success,
        expectation_met,
        error_code: historical_error_code(probe.expected_success, http_success, &validation),
        error_message: transport_error.or(validation.error_message),
        body_exists: !bytes.is_empty(),
        body_bytes: bytes.len() as u64,
        body_sha256,
        json_parse_ok: validation.ok,
        record_count: validation.record_count,
        top_level_fields: validation.top_level_fields,
        websocket_frame_count: None,
        websocket_json_frame_count: None,
        websocket_event_types: Vec::new(),
        websocket_pong_received: None,
        websocket_outbound_messages: Vec::new(),
        websocket_frames: Vec::new(),
        before,
        after: RawFileState {
            body_exists: false,
            metadata_exists: false,
            body_bytes: 0,
            body_sha256: None,
        },
        body_path: body_path.display().to_string(),
        metadata_path: metadata_path.display().to_string(),
    };
    write_json(&metadata_path, &sample)?;
    let after = file_state(&body_path, &metadata_path)?;
    if sample.body_sha256 != after.body_sha256 {
        sample.error_code = Some("POLY_RAW_SOURCE_READBACK_SHA_MISMATCH".to_string());
        sample.expectation_met = false;
    }
    sample.after = after;
    write_json(&metadata_path, &sample)?;
    Ok(sample)
}

fn historical_error_code(
    expected_success: bool,
    http_success: bool,
    validation: &HistoricalValidation,
) -> Option<String> {
    if expected_success && !http_success {
        return Some("POLY_RAW_SOURCE_HISTORICAL_HTTP_FAILED".to_string());
    }
    if expected_success && !validation.ok {
        return Some("POLY_RAW_SOURCE_HISTORICAL_FORMAT_INVALID".to_string());
    }
    if !expected_success && !http_success {
        return Some("POLY_RAW_SOURCE_EXPECTED_HTTP_FAILURE".to_string());
    }
    if !expected_success && !validation.ok {
        return Some("POLY_RAW_SOURCE_EXPECTED_FORMAT_FAILURE".to_string());
    }
    if !expected_success && http_success && validation.ok {
        return Some("POLY_RAW_SOURCE_UNEXPECTED_HTTP_SUCCESS".to_string());
    }
    None
}

fn probe(
    name: impl Into<String>,
    endpoint: impl Into<String>,
    url: impl Into<String>,
    docs_url: impl Into<String>,
    format: HistoricalFormat,
    expected_success: bool,
    edge_case: bool,
) -> HistoricalProbe {
    HistoricalProbe {
        name: name.into(),
        endpoint: endpoint.into(),
        url: url.into(),
        docs_url: docs_url.into(),
        format,
        expected_success,
        edge_case,
    }
}

#[derive(Debug, Clone)]
struct HistoricalProbe {
    name: String,
    endpoint: String,
    url: String,
    docs_url: String,
    format: HistoricalFormat,
    expected_success: bool,
    edge_case: bool,
}
