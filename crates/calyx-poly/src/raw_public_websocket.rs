use std::collections::BTreeSet;
use std::fs;
use std::net::TcpStream;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use serde_json::Value;
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{Message, WebSocket};

use crate::raw_public_websocket_probes::PublicWebSocketProbe;
use crate::raw_public_websocket_shape::public_json_shape;
use crate::raw_source_support::{
    file_state, now_unix_ms, sanitize_segment, sha256_hex, write_json,
};
use crate::raw_sources::{RawEndpointSample, RawFileState, RawSourceSamplingRequest};
use crate::raw_websocket_support::{
    connect_with_timeout, is_timeout, remove_stale_body, send_text, set_socket_read_timeout,
};
use crate::{PolyError, Result};

const MAX_PUBLIC_WS_FRAMES: usize = 10;
const MAX_PUBLIC_WS_WAIT_SECS: u64 = 60;
const PUBLIC_WS_READ_TICK: Duration = Duration::from_secs(1);
const PUBLIC_WS_PING_INTERVAL: Duration = Duration::from_secs(5);
const RTDS_KEEPALIVE_MESSAGE: &str = "ping";

pub(crate) fn capture_public_websocket(
    request: &RawSourceSamplingRequest,
    probe: &PublicWebSocketProbe,
) -> Result<RawEndpointSample> {
    let sample_dir = request
        .output_root
        .join("raw")
        .join(&probe.source)
        .join(sanitize_segment(&probe.name));
    let body_path = sample_dir.join("frames.ndjson");
    let metadata_path = sample_dir.join("metadata.json");
    let before = file_state(&body_path, &metadata_path)?;
    fs::create_dir_all(&sample_dir).map_err(|err| {
        PolyError::raw_source(
            "POLY_RAW_SOURCE_PUBLIC_WS_SAMPLE_DIR_CREATE_FAILED",
            format!(
                "create public WebSocket sample dir {}: {err}",
                sample_dir.display()
            ),
        )
    })?;

    let wait_secs = request
        .timeout_secs
        .clamp(1, probe.max_wait_secs.min(MAX_PUBLIC_WS_WAIT_SECS));
    let wait = Duration::from_secs(wait_secs);
    let capture = run_capture(probe, wait);
    websocket_sample(probe, before, body_path, metadata_path, capture)
}

struct PublicCaptureOutput {
    status_code: Option<u16>,
    handshake_success: bool,
    raw_body: Vec<u8>,
    frames: Vec<crate::raw_sources::RawWebSocketFrameState>,
    outbound_messages: Vec<String>,
    json_frame_count: usize,
    event_frame_count: usize,
    event_types: Vec<String>,
    top_level_fields: Vec<String>,
    text_ping_seen: bool,
    timeout_seen: bool,
    error_code: Option<String>,
    error_message: Option<String>,
}

fn run_capture(probe: &PublicWebSocketProbe, wait: Duration) -> PublicCaptureOutput {
    let mut output = PublicCaptureOutput::failed("POLY_RAW_SOURCE_PUBLIC_WS_CONNECT_FAILED", None);
    let connected = connect_with_timeout(&probe.url, wait);
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
    if let Err(err) = set_socket_read_timeout(&mut socket, PUBLIC_WS_READ_TICK) {
        output.error_code = Some(err.code().to_string());
        output.error_message = Some(err.message());
        return output;
    }

    for message in &probe.outbound_messages {
        if let Err(err) = send_text(&mut socket, message) {
            output.error_code = Some("POLY_RAW_SOURCE_PUBLIC_WS_SEND_FAILED".to_string());
            output.error_message = Some(err.message());
            return output;
        }
        output.outbound_messages.push(message.clone());
    }
    if probe.send_client_ping {
        if let Err(err) = send_text(&mut socket, RTDS_KEEPALIVE_MESSAGE) {
            output.error_code = Some("POLY_RAW_SOURCE_PUBLIC_WS_PING_SEND_FAILED".to_string());
            output.error_message = Some(err.message());
            return output;
        }
        output
            .outbound_messages
            .push(RTDS_KEEPALIVE_MESSAGE.to_string());
    }

    let mut fields = BTreeSet::new();
    let mut event_types = BTreeSet::new();
    let deadline = Instant::now() + wait;
    let mut next_ping = Instant::now() + PUBLIC_WS_PING_INTERVAL;
    while output.frames.len() < MAX_PUBLIC_WS_FRAMES && Instant::now() < deadline {
        if probe.send_client_ping && Instant::now() >= next_ping {
            if let Err(err) = send_text(&mut socket, RTDS_KEEPALIVE_MESSAGE) {
                output.error_code = Some("POLY_RAW_SOURCE_PUBLIC_WS_PING_SEND_FAILED".to_string());
                output.error_message = Some(err.message());
                break;
            }
            output
                .outbound_messages
                .push(RTDS_KEEPALIVE_MESSAGE.to_string());
            next_ping += PUBLIC_WS_PING_INTERVAL;
        }
        let message = match socket.read() {
            Ok(message) => message,
            Err(tungstenite::Error::Io(err)) if is_timeout(&err) => {
                output.timeout_seen = true;
                continue;
            }
            Err(tungstenite::Error::ConnectionClosed | tungstenite::Error::AlreadyClosed) => break,
            Err(err) => {
                output.error_code = Some("POLY_RAW_SOURCE_PUBLIC_WS_READ_FAILED".to_string());
                output.error_message = Some(err.to_string());
                break;
            }
        };
        if let Err(err) = record_message(
            &mut socket,
            probe,
            message,
            &mut output,
            &mut fields,
            &mut event_types,
        ) {
            output.error_code = Some(err.code().to_string());
            output.error_message = Some(err.message());
            break;
        }
        if success_condition(probe, &output) {
            break;
        }
    }
    if !success_condition(probe, &output) && Instant::now() >= deadline {
        output.timeout_seen = true;
    }
    output.top_level_fields = fields.into_iter().collect();
    output.event_types = event_types.into_iter().collect();
    output
}

fn websocket_sample(
    probe: &PublicWebSocketProbe,
    before: RawFileState,
    body_path: PathBuf,
    metadata_path: PathBuf,
    capture: PublicCaptureOutput,
) -> Result<RawEndpointSample> {
    if capture.raw_body.is_empty() {
        remove_stale_body(&body_path)?;
    } else {
        fs::write(&body_path, &capture.raw_body).map_err(|err| {
            PolyError::raw_source(
                "POLY_RAW_SOURCE_PUBLIC_WS_BODY_WRITE_FAILED",
                format!("write public WebSocket body {}: {err}", body_path.display()),
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
        source: probe.source.clone(),
        transport: "websocket".to_string(),
        endpoint: probe.endpoint.clone(),
        method: "WEBSOCKET".to_string(),
        url: probe.url.clone(),
        docs_url: probe.docs_url.clone(),
        request_body_exists: false,
        request_body_bytes: 0,
        request_body_sha256: None,
        request_body_path: None,
        expected_success: probe.expected_success,
        edge_case: probe.edge_case,
        status_code: capture.status_code,
        http_success: capture.handshake_success,
        expectation_met,
        error_code: public_websocket_error_code(probe, &capture, success, expectation_met),
        error_message: capture.error_message,
        body_exists: !capture.raw_body.is_empty(),
        body_bytes: capture.raw_body.len() as u64,
        body_sha256,
        json_parse_ok: capture.json_frame_count > 0
            && capture.frames.iter().all(|frame| {
                frame.error_code.as_deref() != Some("POLY_RAW_SOURCE_PUBLIC_WS_JSON_PARSE_FAILED")
            }),
        record_count: Some(capture.frames.len()),
        top_level_fields: capture.top_level_fields,
        websocket_frame_count: Some(capture.frames.len()),
        websocket_json_frame_count: Some(capture.json_frame_count),
        websocket_event_types: capture.event_types,
        websocket_pong_received: Some(capture.text_ping_seen),
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
    probe: &PublicWebSocketProbe,
    message: Message,
    output: &mut PublicCaptureOutput,
    fields: &mut BTreeSet<String>,
    event_types: &mut BTreeSet<String>,
) -> Result<()> {
    match message {
        Message::Text(text) => {
            let text = text.to_string();
            if text == "ping" && probe.respond_to_text_ping {
                send_text(socket, "pong")?;
                output.outbound_messages.push("pong".to_string());
                output.text_ping_seen = true;
            }
            if text == "pong" {
                output.text_ping_seen = true;
            }
            let parsed = serde_json::from_str::<Value>(&text);
            let (event_type, frame_fields) = parsed
                .as_ref()
                .ok()
                .map(public_json_shape)
                .unwrap_or((None, Vec::new()));
            fields.extend(frame_fields);
            if let Some(event_type) = &event_type {
                event_types.insert(event_type.clone());
                if is_data_event_type(event_type) {
                    output.event_frame_count += 1;
                }
            }
            if parsed.is_ok() {
                output.json_frame_count += 1;
            }
            push_frame(
                output,
                "text",
                text.as_bytes(),
                parsed.is_ok(),
                event_type,
                parse_error_code(&text, parsed.is_err()),
            )?;
        }
        Message::Pong(payload) => {
            push_frame(output, "pong", payload.as_ref(), false, None, None)?;
        }
        Message::Ping(payload) => {
            socket.send(Message::Pong(payload.clone())).map_err(|err| {
                PolyError::raw_source(
                    "POLY_RAW_SOURCE_PUBLIC_WS_PONG_SEND_FAILED",
                    format!("respond to server ping: {err}"),
                )
            })?;
            output.text_ping_seen = true;
            push_frame(output, "ping", payload.as_ref(), false, None, None)?;
        }
        Message::Binary(payload) => push_frame(
            output,
            "binary",
            payload.as_ref(),
            false,
            None,
            Some("POLY_RAW_SOURCE_PUBLIC_WS_BINARY_FRAME".to_string()),
        )?,
        Message::Close(_) => push_frame(output, "close", &[], false, None, None)?,
        Message::Frame(_) => push_frame(
            output,
            "frame",
            &[],
            false,
            None,
            Some("POLY_RAW_SOURCE_PUBLIC_WS_UNEXPECTED_RAW_FRAME".to_string()),
        )?,
    }
    Ok(())
}

fn push_frame(
    output: &mut PublicCaptureOutput,
    opcode: &str,
    body: &[u8],
    json_parse_ok: bool,
    event_type: Option<String>,
    error_code: Option<String>,
) -> Result<()> {
    let received_at_unix_ms = now_unix_ms()?;
    if !output.raw_body.is_empty() {
        output.raw_body.push(b'\n');
    }
    output.raw_body.extend_from_slice(body);
    output
        .frames
        .push(crate::raw_sources::RawWebSocketFrameState {
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

fn success_condition(probe: &PublicWebSocketProbe, output: &PublicCaptureOutput) -> bool {
    let event_requirement_met =
        !probe.require_event_frame || output.event_frame_count >= probe.min_event_frames;
    output.handshake_success
        && event_requirement_met
        && output.error_code.is_none()
        && !output
            .event_types
            .iter()
            .any(|event_type| event_type == "rtds_error")
}

fn is_data_event_type(event_type: &str) -> bool {
    event_type != "rtds_error"
}

fn public_websocket_error_code(
    probe: &PublicWebSocketProbe,
    output: &PublicCaptureOutput,
    success: bool,
    expectation_met: bool,
) -> Option<String> {
    if let Some(code) = &output.error_code {
        return Some(code.clone());
    }
    if expectation_met && !probe.expected_success {
        if output
            .event_types
            .iter()
            .any(|event_type| event_type == "rtds_error")
        {
            return Some("POLY_RAW_SOURCE_EXPECTED_PUBLIC_WS_ERROR_FRAME".to_string());
        }
        return Some(if output.timeout_seen {
            "POLY_RAW_SOURCE_EXPECTED_PUBLIC_WS_TIMEOUT".to_string()
        } else {
            "POLY_RAW_SOURCE_EXPECTED_PUBLIC_WS_FAILURE".to_string()
        });
    }
    if !expectation_met && !probe.expected_success && success {
        return Some("POLY_RAW_SOURCE_PUBLIC_WS_UNEXPECTED_SUCCESS".to_string());
    }
    if !expectation_met && probe.expected_success && output.timeout_seen {
        return Some("POLY_RAW_SOURCE_PUBLIC_WS_TIMEOUT".to_string());
    }
    if !expectation_met && probe.expected_success {
        return Some("POLY_RAW_SOURCE_PUBLIC_WS_REQUIRED_SAMPLE_FAILED".to_string());
    }
    None
}

fn parse_error_code(text: &str, parse_failed: bool) -> Option<String> {
    if parse_failed && !text.is_empty() && text != "ping" && text != "pong" {
        Some("POLY_RAW_SOURCE_PUBLIC_WS_JSON_PARSE_FAILED".to_string())
    } else {
        None
    }
}

impl PublicCaptureOutput {
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
            text_ping_seen: false,
            timeout_seen: false,
            error_code: Some(code.into()),
            error_message: message,
        }
    }
}
