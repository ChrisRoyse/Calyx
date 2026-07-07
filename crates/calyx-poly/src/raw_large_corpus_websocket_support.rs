use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use serde::Serialize;
use serde_json::Value;

use crate::raw_large_corpus_websocket_plan::{WebSocketCapturePlan, WebSocketExpectation};
use crate::raw_sources::RawFileState;
use crate::{PolyError, Result};

const WS_BODY_SCHEMA_VERSION: &str = "poly.large_corpus.websocket.body.v1";
const WS_REQUEST_SCHEMA_VERSION: &str = "poly.large_corpus.websocket.request.v1";

pub(crate) fn write_websocket_body(
    path: &Path,
    plan: &WebSocketCapturePlan,
    capture: &WebSocketCaptureOutput,
) -> Result<Vec<u8>> {
    let body = WebSocketCorpusBody {
        schema_version: WS_BODY_SCHEMA_VERSION,
        dataset: &plan.dataset,
        source: &plan.source,
        endpoint: &plan.endpoint,
        url: &plan.url,
        json_values: &capture.json_values,
        raw_frames: &capture.frames,
        outbound_messages: &capture.outbound_messages,
        event_types: &capture.event_types,
        no_payload_window: capture.data_event_count == 0,
        timeout_seen: capture.timeout_seen,
        error_code: capture.error_code.as_deref(),
        error_message: capture.error_message.as_deref(),
    };
    let bytes = serde_json::to_vec_pretty(&body).map_err(|err| {
        PolyError::raw_source(
            "POLY_LARGE_CORPUS_WS_BODY_ENCODE_FAILED",
            format!("encode WebSocket corpus body {}: {err}", path.display()),
        )
    })?;
    fs::write(path, &bytes).map_err(|err| {
        PolyError::raw_source(
            "POLY_LARGE_CORPUS_WS_BODY_WRITE_FAILED",
            format!("write WebSocket corpus body {}: {err}", path.display()),
        )
    })?;
    Ok(bytes)
}

pub(crate) fn persist_websocket_request(
    path: &Path,
    plan: &WebSocketCapturePlan,
) -> Result<Vec<u8>> {
    let record = WebSocketRequestRecord {
        schema_version: WS_REQUEST_SCHEMA_VERSION,
        url: &plan.url,
        outbound_messages: &plan.outbound_messages,
        heartbeat_message: plan.heartbeat_message,
        send_initial_heartbeat: plan.send_initial_heartbeat,
        respond_to_text_ping: plan.respond_to_text_ping,
        expectation: plan.expectation.as_str(),
    };
    let bytes = serde_json::to_vec_pretty(&record).map_err(|err| {
        PolyError::raw_source(
            "POLY_LARGE_CORPUS_WS_REQUEST_ENCODE_FAILED",
            format!("encode WebSocket request {}: {err}", path.display()),
        )
    })?;
    fs::write(path, &bytes).map_err(|err| {
        PolyError::raw_source(
            "POLY_LARGE_CORPUS_WS_REQUEST_WRITE_FAILED",
            format!("write WebSocket request {}: {err}", path.display()),
        )
    })?;
    let readback = fs::read(path).map_err(|err| {
        PolyError::raw_source(
            "POLY_LARGE_CORPUS_WS_REQUEST_READBACK_FAILED",
            format!("read WebSocket request {}: {err}", path.display()),
        )
    })?;
    if readback != bytes {
        return Err(PolyError::raw_source(
            "POLY_LARGE_CORPUS_WS_REQUEST_READBACK_MISMATCH",
            format!("request readback mismatch at {}", path.display()),
        ));
    }
    Ok(bytes)
}

pub(crate) fn expectation_met(
    plan: &WebSocketCapturePlan,
    output: &WebSocketCaptureOutput,
) -> bool {
    if !output.handshake_success || output.error_code.is_some() {
        return false;
    }
    match plan.expectation {
        WebSocketExpectation::DataEvent => output.data_event_count >= plan.min_data_events,
        WebSocketExpectation::HandshakeWindow => true,
        WebSocketExpectation::ErrorFrame => output.event_type_set.contains("rtds_error"),
        WebSocketExpectation::NoDataEvent => {
            output.data_event_count == 0 && !output.event_type_set.contains("rtds_error")
        }
    }
}

pub(crate) fn parse_error_code(json_parse_ok: bool) -> Option<String> {
    (!json_parse_ok).then(|| "POLY_LARGE_CORPUS_WS_JSON_PARSE_FAILED".to_string())
}

pub(crate) fn empty_state() -> RawFileState {
    RawFileState {
        body_exists: false,
        metadata_exists: false,
        body_bytes: 0,
        body_sha256: None,
    }
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct LargeWebSocketFrame {
    pub(crate) direction: String,
    pub(crate) opcode: String,
    pub(crate) received_at_unix_ms: u128,
    pub(crate) body_bytes: u64,
    pub(crate) body_sha256: Option<String>,
    pub(crate) raw_text: Option<String>,
    pub(crate) json_parse_ok: bool,
    pub(crate) event_type: Option<String>,
    pub(crate) error_code: Option<String>,
}

pub(crate) struct WebSocketCaptureOutput {
    pub(crate) status_code: Option<u16>,
    pub(crate) handshake_success: bool,
    pub(crate) json_values: Vec<Value>,
    pub(crate) frames: Vec<LargeWebSocketFrame>,
    pub(crate) outbound_messages: Vec<String>,
    pub(crate) json_frame_count: usize,
    pub(crate) data_event_count: usize,
    pub(crate) event_type_set: BTreeSet<String>,
    pub(crate) event_types: Vec<String>,
    pub(crate) raw_frame_bytes: usize,
    pub(crate) timeout_seen: bool,
    pub(crate) error_code: Option<String>,
    pub(crate) error_message: Option<String>,
}

impl WebSocketCaptureOutput {
    pub(crate) fn failed(code: impl Into<String>, message: Option<String>) -> Self {
        Self {
            status_code: None,
            handshake_success: false,
            json_values: Vec::new(),
            frames: Vec::new(),
            outbound_messages: Vec::new(),
            json_frame_count: 0,
            data_event_count: 0,
            event_type_set: BTreeSet::new(),
            event_types: Vec::new(),
            raw_frame_bytes: 0,
            timeout_seen: false,
            error_code: Some(code.into()),
            error_message: message,
        }
    }
}

#[derive(Debug, Serialize)]
struct WebSocketCorpusBody<'a> {
    schema_version: &'static str,
    dataset: &'a str,
    source: &'a str,
    endpoint: &'a str,
    url: &'a str,
    json_values: &'a [Value],
    raw_frames: &'a [LargeWebSocketFrame],
    outbound_messages: &'a [String],
    event_types: &'a [String],
    no_payload_window: bool,
    timeout_seen: bool,
    error_code: Option<&'a str>,
    error_message: Option<&'a str>,
}

#[derive(Debug, Serialize)]
struct WebSocketRequestRecord<'a> {
    schema_version: &'static str,
    url: &'a str,
    outbound_messages: &'a [String],
    heartbeat_message: Option<&'static str>,
    send_initial_heartbeat: bool,
    respond_to_text_ping: bool,
    expectation: &'static str,
}
