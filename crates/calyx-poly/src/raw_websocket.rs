use std::collections::BTreeSet;
use std::fs;
use std::net::TcpStream;
use std::path::PathBuf;
use std::time::Duration;

use calyx_core::Clock;
use serde_json::{Value, json};
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{Message, WebSocket};

use crate::raw_source_support::{
    file_state, now_unix_ms, sanitize_segment, sha256_hex, write_json,
};
use crate::raw_sources::{
    RawEndpointSample, RawFileState, RawJoinMap, RawSourceSamplingRequest, RawWebSocketFrameState,
};
use crate::raw_websocket_support::{
    connect_with_timeout, is_timeout, json_shape, remove_stale_body, send_text,
};
use crate::{PolyError, Result};

const MARKET_WS_URL: &str = "wss://ws-subscriptions-clob.polymarket.com/ws/market";
const MARKET_WS_DOCS_URL: &str = "https://docs.polymarket.com/market-data/websocket/market-channel";
const MAX_WS_FRAMES: usize = 12;
const MAX_WS_WAIT_SECS: u64 = 8;

#[derive(Debug, Clone)]
pub(crate) struct WebSocketProbe {
    name: String,
    subscription: String,
    expected_success: bool,
    edge_case: bool,
    min_event_frames: usize,
    require_pong: bool,
}

pub(crate) fn market_websocket_probes(join: &RawJoinMap) -> Vec<WebSocketProbe> {
    let Some(token) = &join.token_id else {
        return Vec::new();
    };
    let mut assets = vec![token.clone()];
    if let Some(opposite) = &join.opposite_token_id {
        assets.push(opposite.clone());
    }
    vec![
        WebSocketProbe {
            name: "ws_market_books_by_join_tokens".to_string(),
            subscription: json!({
                "assets_ids": assets,
                "type": "market",
                "initial_dump": true,
                "level": 2,
                "custom_feature_enabled": false
            })
            .to_string(),
            expected_success: true,
            edge_case: false,
            min_event_frames: 1,
            require_pong: true,
        },
        WebSocketProbe {
            name: "edge_ws_market_invalid_token_no_custom".to_string(),
            subscription: json!({
                "assets_ids": ["not-a-real-token"],
                "type": "market",
                "custom_feature_enabled": false
            })
            .to_string(),
            expected_success: false,
            edge_case: true,
            min_event_frames: 1,
            require_pong: false,
        },
        WebSocketProbe {
            name: "edge_ws_market_empty_assets_no_custom".to_string(),
            subscription: json!({
                "assets_ids": [],
                "type": "market",
                "custom_feature_enabled": false
            })
            .to_string(),
            expected_success: false,
            edge_case: true,
            min_event_frames: 1,
            require_pong: false,
        },
        WebSocketProbe {
            name: "edge_ws_market_malformed_subscription".to_string(),
            subscription: "{not-json".to_string(),
            expected_success: false,
            edge_case: true,
            min_event_frames: 1,
            require_pong: false,
        },
    ]
}

pub(crate) fn capture_market_websocket(
    request: &RawSourceSamplingRequest,
    probe: &WebSocketProbe,
    clock: &dyn Clock,
) -> Result<RawEndpointSample> {
    let sample_dir = request
        .output_root
        .join("raw")
        .join("websocket-market")
        .join(sanitize_segment(&probe.name));
    let body_path = sample_dir.join("frames.ndjson");
    let metadata_path = sample_dir.join("metadata.json");
    let before = file_state(&body_path, &metadata_path)?;
    fs::create_dir_all(&sample_dir).map_err(|err| {
        PolyError::raw_source(
            "POLY_RAW_SOURCE_WS_SAMPLE_DIR_CREATE_FAILED",
            format!(
                "create WebSocket sample dir {}: {err}",
                sample_dir.display()
            ),
        )
    })?;

    let wait = Duration::from_secs(request.timeout_secs.clamp(1, MAX_WS_WAIT_SECS));
    let capture = run_capture(probe, wait, clock);
    let sample = websocket_sample(probe, before, body_path, metadata_path, capture)?;
    Ok(sample)
}

struct CaptureOutput {
    status_code: Option<u16>,
    handshake_success: bool,
    raw_body: Vec<u8>,
    frames: Vec<RawWebSocketFrameState>,
    outbound_messages: Vec<String>,
    json_frame_count: usize,
    event_frame_count: usize,
    event_types: Vec<String>,
    top_level_fields: Vec<String>,
    pong_received: bool,
    timeout_seen: bool,
    error_code: Option<String>,
    error_message: Option<String>,
}

fn run_capture(probe: &WebSocketProbe, wait: Duration, clock: &dyn Clock) -> CaptureOutput {
    let mut output = CaptureOutput::failed("POLY_RAW_SOURCE_WS_CONNECT_FAILED", None);
    let connected = connect_with_timeout(MARKET_WS_URL, wait);
    let (mut socket, response) = match connected {
        Ok(connected) => connected,
        Err(err) => {
            output.error_message = Some(err.message());
            return output;
        }
    };
    output.status_code = Some(response.status().as_u16());
    output.handshake_success = true;
    output.error_code = None;
    output.error_message = None;

    if let Err(err) = send_text(&mut socket, &probe.subscription) {
        output.error_code = Some("POLY_RAW_SOURCE_WS_SUBSCRIBE_FAILED".to_string());
        output.error_message = Some(err.message());
        return output;
    }
    output.outbound_messages.push(probe.subscription.clone());
    if probe.require_pong {
        if let Err(err) = send_text(&mut socket, "PING") {
            output.error_code = Some("POLY_RAW_SOURCE_WS_PING_SEND_FAILED".to_string());
            output.error_message = Some(err.message());
            return output;
        }
        output.outbound_messages.push("PING".to_string());
    }

    let mut fields = BTreeSet::new();
    let mut event_types = BTreeSet::new();
    for _ in 0..MAX_WS_FRAMES {
        let message = match socket.read() {
            Ok(message) => message,
            Err(tungstenite::Error::Io(err)) if is_timeout(&err) => {
                output.timeout_seen = true;
                break;
            }
            Err(tungstenite::Error::ConnectionClosed | tungstenite::Error::AlreadyClosed) => break,
            Err(err) => {
                output.error_code = Some("POLY_RAW_SOURCE_WS_READ_FAILED".to_string());
                output.error_message = Some(err.to_string());
                break;
            }
        };
        if let Err(err) = record_message(
            &mut socket,
            message,
            &mut output,
            &mut fields,
            &mut event_types,
            clock,
        ) {
            output.error_code = Some(err.code().to_string());
            output.error_message = Some(err.message());
            break;
        }
        if success_condition(probe, &output) {
            break;
        }
    }
    output.top_level_fields = fields.into_iter().collect();
    output.event_types = event_types.into_iter().collect();
    output
}

fn websocket_sample(
    probe: &WebSocketProbe,
    before: RawFileState,
    body_path: PathBuf,
    metadata_path: PathBuf,
    capture: CaptureOutput,
) -> Result<RawEndpointSample> {
    if capture.raw_body.is_empty() {
        remove_stale_body(&body_path)?;
    } else {
        fs::write(&body_path, &capture.raw_body).map_err(|err| {
            PolyError::raw_source(
                "POLY_RAW_SOURCE_WS_BODY_WRITE_FAILED",
                format!("write WebSocket body {}: {err}", body_path.display()),
            )
        })?;
    }
    let body_sha256 = (!capture.raw_body.is_empty()).then(|| sha256_hex(&capture.raw_body));
    let success = success_condition(probe, &capture);
    let expectation_met = if probe.expected_success {
        success
    } else {
        capture.handshake_success && !success
    };
    let mut sample = RawEndpointSample {
        name: probe.name.clone(),
        source: "websocket-market".to_string(),
        transport: "websocket".to_string(),
        endpoint: "market".to_string(),
        method: "WEBSOCKET".to_string(),
        url: MARKET_WS_URL.to_string(),
        docs_url: MARKET_WS_DOCS_URL.to_string(),
        request_body_exists: false,
        request_body_bytes: 0,
        request_body_sha256: None,
        request_body_path: None,
        expected_success: probe.expected_success,
        edge_case: probe.edge_case,
        status_code: capture.status_code,
        http_success: capture.handshake_success,
        expectation_met,
        error_code: websocket_error_code(probe, &capture, success, expectation_met),
        error_message: capture.error_message,
        body_exists: !capture.raw_body.is_empty(),
        body_bytes: capture.raw_body.len() as u64,
        body_sha256,
        json_parse_ok: capture.json_frame_count > 0
            && capture.frames.iter().all(|frame| {
                frame.error_code.as_deref() != Some("POLY_RAW_SOURCE_WS_JSON_PARSE_FAILED")
            }),
        record_count: Some(capture.frames.len()),
        top_level_fields: capture.top_level_fields,
        websocket_frame_count: Some(capture.frames.len()),
        websocket_json_frame_count: Some(capture.json_frame_count),
        websocket_event_types: capture.event_types,
        websocket_pong_received: Some(capture.pong_received),
        websocket_outbound_messages: capture.outbound_messages,
        websocket_frames: capture.frames,
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

fn record_message(
    socket: &mut WebSocket<MaybeTlsStream<TcpStream>>,
    message: Message,
    output: &mut CaptureOutput,
    fields: &mut BTreeSet<String>,
    event_types: &mut BTreeSet<String>,
    clock: &dyn Clock,
) -> Result<()> {
    match message {
        Message::Text(text) => {
            let text = text.to_string();
            let parsed = serde_json::from_str::<Value>(&text);
            let (event_type, frame_fields) = parsed
                .as_ref()
                .ok()
                .map(json_shape)
                .unwrap_or((None, Vec::new()));
            fields.extend(frame_fields);
            if let Some(event_type) = &event_type {
                event_types.insert(event_type.clone());
                output.event_frame_count += 1;
            }
            if parsed.is_ok() {
                output.json_frame_count += 1;
            } else if text == "PONG" {
                output.pong_received = true;
            }
            push_frame(
                output,
                "text",
                text.as_bytes(),
                parsed.is_ok(),
                event_type,
                parse_error_code(&text, parsed.is_err()),
                clock,
            )?;
        }
        Message::Pong(payload) => {
            output.pong_received = true;
            push_frame(output, "pong", payload.as_ref(), false, None, None, clock)?;
        }
        Message::Ping(payload) => {
            socket.send(Message::Pong(payload.clone())).map_err(|err| {
                PolyError::raw_source(
                    "POLY_RAW_SOURCE_WS_PONG_SEND_FAILED",
                    format!("respond to server ping: {err}"),
                )
            })?;
            push_frame(output, "ping", payload.as_ref(), false, None, None, clock)?;
        }
        Message::Binary(payload) => {
            push_frame(
                output,
                "binary",
                payload.as_ref(),
                false,
                None,
                Some("POLY_RAW_SOURCE_WS_BINARY_FRAME".to_string()),
                clock,
            )?;
        }
        Message::Close(_) => {
            push_frame(output, "close", &[], false, None, None, clock)?;
        }
        Message::Frame(_) => {
            push_frame(
                output,
                "frame",
                &[],
                false,
                None,
                Some("POLY_RAW_SOURCE_WS_UNEXPECTED_RAW_FRAME".to_string()),
                clock,
            )?;
        }
    }
    Ok(())
}

fn push_frame(
    output: &mut CaptureOutput,
    opcode: &str,
    body: &[u8],
    json_parse_ok: bool,
    event_type: Option<String>,
    error_code: Option<String>,
    clock: &dyn Clock,
) -> Result<()> {
    let received_at_unix_ms = now_unix_ms(clock);
    if !output.raw_body.is_empty() {
        output.raw_body.push(b'\n');
    }
    output.raw_body.extend_from_slice(body);
    output.frames.push(RawWebSocketFrameState {
        direction: "inbound".to_string(),
        opcode: opcode.to_string(),
        received_at_unix_ms,
        body_bytes: body.len() as u64,
        body_sha256: (!body.is_empty()).then(|| sha256_hex(body)),
        json_parse_ok,
        event_type,
        error_code,
    });
    Ok(())
}

fn success_condition(probe: &WebSocketProbe, output: &CaptureOutput) -> bool {
    output.handshake_success
        && output.event_frame_count >= probe.min_event_frames
        && (!probe.require_pong || output.pong_received)
        && output.error_code.is_none()
}

fn websocket_error_code(
    probe: &WebSocketProbe,
    output: &CaptureOutput,
    success: bool,
    expectation_met: bool,
) -> Option<String> {
    if let Some(code) = &output.error_code {
        return Some(code.clone());
    }
    if expectation_met && !probe.expected_success {
        return Some(if output.timeout_seen {
            "POLY_RAW_SOURCE_EXPECTED_WS_TIMEOUT".to_string()
        } else {
            "POLY_RAW_SOURCE_EXPECTED_WS_FAILURE".to_string()
        });
    }
    if !expectation_met && !probe.expected_success && success {
        return Some("POLY_RAW_SOURCE_WS_UNEXPECTED_SUCCESS".to_string());
    }
    if !expectation_met && probe.expected_success && output.timeout_seen {
        return Some("POLY_RAW_SOURCE_WS_TIMEOUT".to_string());
    }
    if !expectation_met && probe.expected_success && !output.pong_received {
        return Some("POLY_RAW_SOURCE_WS_PONG_MISSING".to_string());
    }
    if !expectation_met && probe.expected_success {
        return Some("POLY_RAW_SOURCE_WS_REQUIRED_SAMPLE_FAILED".to_string());
    }
    None
}

fn parse_error_code(text: &str, parse_failed: bool) -> Option<String> {
    if parse_failed && text != "PONG" {
        Some("POLY_RAW_SOURCE_WS_JSON_PARSE_FAILED".to_string())
    } else {
        None
    }
}

impl CaptureOutput {
    fn failed(code: impl Into<String>, message: Option<String>) -> Self {
        Self {
            status_code: None,
            handshake_success: false,
            raw_body: Vec::new(),
            frames: Vec::new(),
            outbound_messages: Vec::new(),
            json_frame_count: 0,
            event_frame_count: 0,
            event_types: Vec::new(),
            top_level_fields: Vec::new(),
            pong_received: false,
            timeout_seen: false,
            error_code: Some(code.into()),
            error_message: message,
        }
    }
}
