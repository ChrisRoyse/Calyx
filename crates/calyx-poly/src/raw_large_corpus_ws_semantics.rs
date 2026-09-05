use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::raw_large_corpus::{
    LargeCorpusEdgeCase, LargeCorpusFailure, LargeCorpusPage, LargeCorpusReadbackReport,
};
use crate::raw_large_corpus_support::failure;
use crate::raw_source_support::{sha256_hex, write_json};
use crate::raw_sources::RawFileState;
use crate::{PolyError, Result};

const SCHEMA_VERSION: &str = "poly.large_corpus.websocket_runtime_semantics.v1";
const RELATIVE_PATH: &str = "schema-observations/websocket-runtime-semantics.json";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LargeCorpusWebSocketRuntimeSemanticsReport {
    pub schema_version: String,
    pub source_of_truth: String,
    pub observation_count: usize,
    pub observations: Vec<LargeCorpusWebSocketRuntimeSemanticsObservation>,
    pub passed: bool,
    pub failure: Option<LargeCorpusFailure>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LargeCorpusWebSocketRuntimeSemanticsObservation {
    pub sample_name: String,
    pub source: String,
    pub endpoint: String,
    pub method: String,
    pub docs_url: String,
    pub request_case: String,
    pub expected_runtime_semantics: String,
    pub actual_status_code: Option<u16>,
    pub actual_body_shape: String,
    pub actual_body_fields: Vec<String>,
    pub websocket_frame_count: Option<usize>,
    pub websocket_json_frame_count: Option<usize>,
    pub websocket_event_types: Vec<String>,
    pub no_payload_window: bool,
    pub semantics_match: bool,
    pub failure_code: Option<String>,
    pub schema_implication: String,
    pub request_body_path: Option<String>,
    pub request_body_sha256: Option<String>,
    pub body_path: String,
    pub metadata_path: String,
    pub body_bytes: u64,
    pub body_sha256: Option<String>,
    pub metadata_sha256: String,
    pub before: RawFileState,
    pub after: RawFileState,
}

pub(crate) fn build_websocket_runtime_semantics(
    pages: &[LargeCorpusPage],
    edges: &[LargeCorpusEdgeCase],
) -> Result<LargeCorpusWebSocketRuntimeSemanticsReport> {
    let mut observations = Vec::new();
    for page in pages {
        if let Some(expectation) = page_expectation(page) {
            observations.push(page_observation(page, expectation)?);
        }
    }
    for edge in edges {
        if let Some(expectation) = edge_expectation(edge) {
            observations.push(edge_observation(edge, expectation)?);
        }
    }
    let failure = runtime_semantics_failure(&observations);
    Ok(LargeCorpusWebSocketRuntimeSemanticsReport {
        schema_version: SCHEMA_VERSION.to_string(),
        source_of_truth: "persisted large-corpus WebSocket request, body, and metadata files"
            .to_string(),
        observation_count: observations.len(),
        observations,
        passed: failure.is_none(),
        failure,
    })
}

pub(crate) fn write_websocket_runtime_semantics(
    root: &Path,
    report: &LargeCorpusWebSocketRuntimeSemanticsReport,
) -> Result<PathBuf> {
    let path = root.join(RELATIVE_PATH);
    write_json(&path, report)?;
    Ok(path)
}

pub(crate) fn check_websocket_runtime_semantics_artifact(
    root: &Path,
    report: &mut LargeCorpusReadbackReport,
) {
    let path = root.join(RELATIVE_PATH);
    if !path.exists() {
        report.parse_failures.push(format!(
            "{} missing required WebSocket runtime semantics artifact",
            path.display()
        ));
        return;
    }
    report.checked_file_count += 1;
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(err) => {
            report
                .missing_files
                .push(format!("{}: {err}", path.display()));
            return;
        }
    };
    let semantics =
        match serde_json::from_slice::<LargeCorpusWebSocketRuntimeSemanticsReport>(&bytes) {
            Ok(value) => value,
            Err(err) => {
                report
                    .parse_failures
                    .push(format!("{}: {err}", path.display()));
                return;
            }
        };
    if semantics.observation_count != semantics.observations.len() {
        report.parse_failures.push(format!(
            "{} observation_count {} did not equal observations len {}",
            path.display(),
            semantics.observation_count,
            semantics.observations.len()
        ));
    }
    if semantics.observations.len() < 3 {
        report.parse_failures.push(format!(
            "{} had fewer than 3 WebSocket runtime observations",
            path.display()
        ));
    }
    if !semantics.passed {
        report.parse_failures.push(format!(
            "{} recorded failed WebSocket runtime semantics: {:?}",
            path.display(),
            semantics.failure
        ));
    }
    for observation in &semantics.observations {
        check_observation_file_hashes(observation, report);
    }
}

fn page_expectation(page: &LargeCorpusPage) -> Option<RuntimeExpectation> {
    match page.dataset.as_str() {
        "websocket_market_books_large" => Some(RuntimeExpectation {
            request_case: "market channel assets subscription",
            expected_runtime_semantics: "HTTP upgrade 101 with asset-scoped book frame",
            schema_implication: "Market book windows are asset-scoped event frames, not REST snapshots.",
            failure_code: "POLY_LARGE_CORPUS_WS_MARKET_BOOK_SEMANTICS_CHANGED",
            matches: has_event(page, "book") && !page.no_payload_window,
        }),
        "websocket_rtds_crypto_prices_large" => Some(RuntimeExpectation {
            request_case: "RTDS crypto_prices update subscription",
            expected_runtime_semantics: "crypto_prices update payload is emitted",
            schema_implication: "RTDS crypto payloads are sampled and can drive topic-specific schema.",
            failure_code: "POLY_LARGE_CORPUS_WS_RTDS_CRYPTO_SEMANTICS_CHANGED",
            matches: has_event(page, "crypto_prices:update"),
        }),
        "websocket_rtds_crypto_chainlink_large" => Some(RuntimeExpectation {
            request_case: "RTDS crypto_prices_chainlink update subscription",
            expected_runtime_semantics: "crypto_prices_chainlink update payload is emitted",
            schema_implication: "RTDS Chainlink symbols are sampled separately from exchange symbols.",
            failure_code: "POLY_LARGE_CORPUS_WS_RTDS_CHAINLINK_SEMANTICS_CHANGED",
            matches: has_event(page, "crypto_prices_chainlink:update"),
        }),
        _ => None,
    }
}

fn edge_expectation(edge: &LargeCorpusEdgeCase) -> Option<RuntimeExpectation> {
    match edge.name.as_str() {
        "edge_ws_market_unsubscribe_first_message_data_large" => Some(RuntimeExpectation {
            request_case: "market channel first message operation=unsubscribe with valid asset",
            expected_runtime_semantics: "unsubscribe-first can emit a book data frame",
            schema_implication: "Do not model unsubscribe as a pure quiet control action without runtime evidence.",
            failure_code: "POLY_LARGE_CORPUS_WS_UNSUBSCRIBE_SEMANTICS_CHANGED",
            matches: has_edge_event(edge, "book") && !edge.no_payload_window,
        }),
        "edge_ws_rtds_malformed_subscription_no_payload_large" => Some(RuntimeExpectation {
            request_case: "RTDS malformed text subscription",
            expected_runtime_semantics: "malformed text produced a quiet no-payload window",
            schema_implication: "Malformed RTDS inputs can be quiet windows; error-frame and timeout states must differ.",
            failure_code: "POLY_LARGE_CORPUS_WS_RTDS_MALFORMED_SEMANTICS_CHANGED",
            matches: edge.no_payload_window && edge.websocket_event_types.is_empty(),
        }),
        "edge_ws_rtds_unknown_topic_large" => Some(RuntimeExpectation {
            request_case: "RTDS unknown topic subscription",
            expected_runtime_semantics: "unknown topic emits a structured rtds_error frame",
            schema_implication: "RTDS structured errors must be stored separately from no-payload windows.",
            failure_code: "POLY_LARGE_CORPUS_WS_RTDS_UNKNOWN_TOPIC_SEMANTICS_CHANGED",
            matches: has_edge_event(edge, "rtds_error"),
        }),
        "edge_ws_rtds_equity_aapl_blocked_runtime_large" => Some(RuntimeExpectation {
            request_case: "RTDS documented equity_prices AAPL subscription",
            expected_runtime_semantics: "equity_prices remains a no-payload blocked-runtime window",
            schema_implication: "Do not infer RTDS equity payload tables until real topic=equity_prices bytes exist.",
            failure_code: "POLY_LARGE_CORPUS_WS_RTDS_EQUITY_SEMANTICS_CHANGED",
            matches: edge.no_payload_window && edge.websocket_event_types.is_empty(),
        }),
        _ => None,
    }
}

fn page_observation(
    page: &LargeCorpusPage,
    expectation: RuntimeExpectation,
) -> Result<LargeCorpusWebSocketRuntimeSemanticsObservation> {
    observation(ObservationInput {
        sample_name: page.dataset.clone(),
        source: page.source.clone(),
        endpoint: page.endpoint.clone(),
        method: page.method.clone(),
        docs_url: page.docs_url.clone(),
        request_path: page.request_path.clone(),
        request_sha: page.request_body_sha256.clone(),
        status_code: page.status_code,
        frame_count: page.websocket_frame_count,
        json_frame_count: page.websocket_json_frame_count,
        event_types: page.websocket_event_types.clone(),
        no_payload_window: page.no_payload_window,
        body_path: page.body_path.clone(),
        metadata_path: page.metadata_path.clone(),
        body_sha: page.body_sha256.clone(),
        before: page.before.clone(),
        after: page.after.clone(),
        expectation,
    })
}

fn edge_observation(
    edge: &LargeCorpusEdgeCase,
    expectation: RuntimeExpectation,
) -> Result<LargeCorpusWebSocketRuntimeSemanticsObservation> {
    observation(ObservationInput {
        sample_name: edge.name.clone(),
        source: "websocket-edge".to_string(),
        endpoint: edge.name.clone(),
        method: edge.method.clone(),
        docs_url: String::new(),
        request_path: edge.request_path.clone(),
        request_sha: edge.request_body_sha256.clone(),
        status_code: edge.status_code,
        frame_count: edge.websocket_frame_count,
        json_frame_count: edge.websocket_json_frame_count,
        event_types: edge.websocket_event_types.clone(),
        no_payload_window: edge.no_payload_window,
        body_path: edge.body_path.clone(),
        metadata_path: edge.metadata_path.clone(),
        body_sha: edge.body_sha256.clone(),
        before: edge.before.clone(),
        after: edge.after.clone(),
        expectation,
    })
}

struct ObservationInput {
    sample_name: String,
    source: String,
    endpoint: String,
    method: String,
    docs_url: String,
    request_path: Option<String>,
    request_sha: Option<String>,
    status_code: Option<u16>,
    frame_count: Option<usize>,
    json_frame_count: Option<usize>,
    event_types: Vec<String>,
    no_payload_window: bool,
    body_path: String,
    metadata_path: String,
    body_sha: Option<String>,
    before: RawFileState,
    after: RawFileState,
    expectation: RuntimeExpectation,
}

fn observation(input: ObservationInput) -> Result<LargeCorpusWebSocketRuntimeSemanticsObservation> {
    verify_optional_file_hash(&input.request_path, &input.request_sha, "request")?;
    let (body_bytes, actual_body_sha, body_value) = read_json_file(&input.body_path, "body")?;
    if input.body_sha.as_deref() != Some(actual_body_sha.as_str()) {
        return Err(PolyError::raw_source(
            "POLY_LARGE_CORPUS_WS_RUNTIME_BODY_SHA_MISMATCH",
            format!(
                "runtime semantics body hash mismatch for {}: expected {:?} actual {actual_body_sha}",
                input.body_path, input.body_sha
            ),
        ));
    }
    let (_, metadata_sha, _) = read_json_file(&input.metadata_path, "metadata")?;
    Ok(LargeCorpusWebSocketRuntimeSemanticsObservation {
        sample_name: input.sample_name,
        source: input.source,
        endpoint: input.endpoint,
        method: input.method,
        docs_url: input.docs_url,
        request_case: input.expectation.request_case.to_string(),
        expected_runtime_semantics: input.expectation.expected_runtime_semantics.to_string(),
        actual_status_code: input.status_code,
        actual_body_shape: body_shape(&body_value),
        actual_body_fields: object_fields(&body_value),
        websocket_frame_count: input.frame_count,
        websocket_json_frame_count: input.json_frame_count,
        websocket_event_types: input.event_types,
        no_payload_window: input.no_payload_window,
        semantics_match: input.expectation.matches,
        failure_code: (!input.expectation.matches)
            .then(|| input.expectation.failure_code.to_string()),
        schema_implication: input.expectation.schema_implication.to_string(),
        request_body_path: input.request_path,
        request_body_sha256: input.request_sha,
        body_path: input.body_path,
        metadata_path: input.metadata_path,
        body_bytes: body_bytes as u64,
        body_sha256: input.body_sha,
        metadata_sha256: metadata_sha,
        before: input.before,
        after: input.after,
    })
}

fn runtime_semantics_failure(
    observations: &[LargeCorpusWebSocketRuntimeSemanticsObservation],
) -> Option<LargeCorpusFailure> {
    if observations.is_empty() {
        return Some(failure(
            "POLY_LARGE_CORPUS_WS_RUNTIME_SEMANTICS_EMPTY",
            "no WebSocket runtime semantics observations were generated",
        ));
    }
    observations
        .iter()
        .find(|item| !item.semantics_match)
        .map(|item| {
            failure(
                item.failure_code.clone().unwrap_or_else(|| {
                    "POLY_LARGE_CORPUS_WS_RUNTIME_SEMANTICS_CHANGED".to_string()
                }),
                format!(
                    "{} did not match expected runtime semantics: {}",
                    item.sample_name, item.expected_runtime_semantics
                ),
            )
        })
}

fn check_observation_file_hashes(
    observation: &LargeCorpusWebSocketRuntimeSemanticsObservation,
    report: &mut LargeCorpusReadbackReport,
) {
    if let Some(path) = &observation.request_body_path {
        check_file_sha(path, &observation.request_body_sha256, report);
    }
    check_file_sha(&observation.body_path, &observation.body_sha256, report);
    check_file_sha(
        &observation.metadata_path,
        &Some(observation.metadata_sha256.clone()),
        report,
    );
}

fn check_file_sha(path: &str, expected: &Option<String>, report: &mut LargeCorpusReadbackReport) {
    report.checked_file_count += 1;
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(err) => {
            report.missing_files.push(format!("{path}: {err}"));
            return;
        }
    };
    let actual = sha256_hex(&bytes);
    if expected.as_deref() != Some(actual.as_str()) {
        report
            .sha_mismatches
            .push(format!("{path}: expected {:?} actual {actual}", expected));
    }
}

fn verify_optional_file_hash(
    path: &Option<String>,
    expected: &Option<String>,
    label: &str,
) -> Result<()> {
    let Some(path) = path else {
        return Ok(());
    };
    let bytes = fs::read(path).map_err(|err| {
        PolyError::raw_source(
            "POLY_LARGE_CORPUS_WS_RUNTIME_FILE_READ_FAILED",
            format!("read {label} file {path}: {err}"),
        )
    })?;
    let actual = sha256_hex(&bytes);
    if expected.as_deref() != Some(actual.as_str()) {
        return Err(PolyError::raw_source(
            "POLY_LARGE_CORPUS_WS_RUNTIME_FILE_SHA_MISMATCH",
            format!(
                "{label} file {path} expected {:?} actual {actual}",
                expected
            ),
        ));
    }
    Ok(())
}

fn read_json_file(path: &str, label: &str) -> Result<(usize, String, Value)> {
    let bytes = fs::read(path).map_err(|err| {
        PolyError::raw_source(
            "POLY_LARGE_CORPUS_WS_RUNTIME_FILE_READ_FAILED",
            format!("read {label} file {path}: {err}"),
        )
    })?;
    let sha = sha256_hex(&bytes);
    let value = serde_json::from_slice::<Value>(&bytes).map_err(|err| {
        PolyError::raw_source(
            "POLY_LARGE_CORPUS_WS_RUNTIME_FILE_DECODE_FAILED",
            format!("decode {label} file {path}: {err}"),
        )
    })?;
    Ok((bytes.len(), sha, value))
}

fn body_shape(value: &Value) -> String {
    let event_types = value
        .get("event_types")
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(Value::as_str).collect::<Vec<_>>())
        .unwrap_or_default();
    if event_types.contains(&"rtds_error") {
        "websocket_structured_rtds_error".to_string()
    } else if value
        .get("no_payload_window")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        "websocket_no_payload_window".to_string()
    } else if event_types.contains(&"book") {
        "websocket_asset_book_frame".to_string()
    } else if event_types.is_empty() {
        "websocket_no_classified_event".to_string()
    } else {
        "websocket_event_frames".to_string()
    }
}

fn object_fields(value: &Value) -> Vec<String> {
    value
        .as_object()
        .map(|object| object.keys().cloned().collect::<BTreeSet<_>>())
        .unwrap_or_default()
        .into_iter()
        .collect()
}

fn has_event(page: &LargeCorpusPage, event: &str) -> bool {
    page.status_code == Some(101) && page.websocket_event_types.iter().any(|item| item == event)
}

fn has_edge_event(edge: &LargeCorpusEdgeCase, event: &str) -> bool {
    edge.status_code == Some(101) && edge.websocket_event_types.iter().any(|item| item == event)
}

struct RuntimeExpectation {
    request_case: &'static str,
    expected_runtime_semantics: &'static str,
    schema_implication: &'static str,
    failure_code: &'static str,
    matches: bool,
}
