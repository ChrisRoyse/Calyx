use std::fs;
use std::net::TcpStream;
use std::path::Path;
use std::time::{Duration, Instant};

use calyx_core::Clock;
use serde_json::Value;
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{Message, WebSocket};

use crate::raw_large_corpus::{LargeCorpusEdgeCase, LargeCorpusPage, LargeCorpusRequest};
use crate::raw_large_corpus_clob_plan::ClobTarget;
use crate::raw_large_corpus_profile::CorpusRecord;
use crate::raw_large_corpus_support::{collect_records, record_count};
use crate::raw_large_corpus_websocket_plan::{
    WebSocketCapturePlan, WebSocketShape, websocket_edge_plans, websocket_plans,
};
use crate::raw_large_corpus_websocket_support::{
    LargeWebSocketFrame, WebSocketCaptureOutput, empty_state, expectation_met, parse_error_code,
    persist_websocket_request, write_websocket_body,
};
use crate::raw_public_websocket_shape::public_json_shape;
use crate::raw_source_support::{file_state, now_unix_ms, sha256_hex, write_json};
use crate::raw_websocket_support::{
    connect_with_timeout, is_timeout, json_shape, send_text, set_socket_read_timeout,
};
use crate::{PolyError, Result};

pub(crate) fn capture_websocket_market_data(
    request: &LargeCorpusRequest,
    targets: &[ClobTarget],
    pages: &mut Vec<LargeCorpusPage>,
    records: &mut Vec<CorpusRecord>,
    clock: &dyn Clock,
) -> Result<()> {
    for plan in websocket_plans(targets)? {
        let page = capture_websocket_page(request, &plan, clock)?;
        collect_records(
            &page.dataset,
            &page.source,
            Path::new(&page.body_path),
            records,
        )?;
        pages.push(page);
    }
    Ok(())
}

pub(crate) fn capture_websocket_edge_cases(
    request: &LargeCorpusRequest,
    targets: &[ClobTarget],
    clock: &dyn Clock,
) -> Result<Vec<LargeCorpusEdgeCase>> {
    websocket_edge_plans(targets)?
        .iter()
        .map(|plan| capture_websocket_edge(request, plan, clock))
        .collect()
}

fn capture_websocket_page(
    request: &LargeCorpusRequest,
    plan: &WebSocketCapturePlan,
    clock: &dyn Clock,
) -> Result<LargeCorpusPage> {
    let dir = request.output_root.join("raw").join(&plan.dataset);
    let body_path = dir.join("page-000000.json");
    let metadata_path = dir.join("page-000000.metadata.json");
    let request_path = dir.join("page-000000.request.json");
    let before = file_state(&body_path, &metadata_path)?;
    fs::create_dir_all(&dir).map_err(|err| {
        PolyError::raw_source(
            "POLY_LARGE_CORPUS_WS_DIR_CREATE_FAILED",
            format!("create WebSocket corpus dir {}: {err}", dir.display()),
        )
    })?;
    let request_bytes = persist_websocket_request(&request_path, plan)?;
    let capture = run_websocket_capture(plan, request, clock)?;
    let body_bytes = write_websocket_body(&body_path, plan, &capture)?;
    let body_value = serde_json::from_slice::<Value>(&body_bytes).map_err(|err| {
        PolyError::raw_source(
            "POLY_LARGE_CORPUS_WS_BODY_READBACK_DECODE_FAILED",
            format!(
                "decode just-written WebSocket body {}: {err}",
                body_path.display()
            ),
        )
    })?;
    let no_payload_window = capture.data_event_count == 0;
    let mut page = LargeCorpusPage {
        dataset: plan.dataset.clone(),
        source: plan.source.clone(),
        endpoint: plan.endpoint.clone(),
        method: "WEBSOCKET".to_string(),
        docs_url: plan.docs_url.clone(),
        page_index: 0,
        url: plan.url.clone(),
        request_path: Some(request_path.display().to_string()),
        request_body_bytes: request_bytes.len() as u64,
        request_body_sha256: Some(sha256_hex(&request_bytes)),
        status_code: capture.status_code,
        http_success: capture.handshake_success,
        expectation_met: expectation_met(plan, &capture),
        record_count: record_count(&body_value),
        stop_reason: Some(plan.expectation.as_str().to_string()),
        body_path: body_path.display().to_string(),
        metadata_path: metadata_path.display().to_string(),
        body_format: "json".to_string(),
        body_bytes: body_bytes.len() as u64,
        body_sha256: Some(sha256_hex(&body_bytes)),
        json_parse_ok: true,
        websocket_frame_count: Some(capture.frames.len()),
        websocket_json_frame_count: Some(capture.json_frame_count),
        websocket_event_types: capture.event_types.clone(),
        no_payload_window,
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

fn capture_websocket_edge(
    request: &LargeCorpusRequest,
    plan: &WebSocketCapturePlan,
    clock: &dyn Clock,
) -> Result<LargeCorpusEdgeCase> {
    let dir = request.output_root.join("edge").join(&plan.dataset);
    let body_path = dir.join("body.json");
    let metadata_path = dir.join("metadata.json");
    let request_path = dir.join("request.json");
    let before = file_state(&body_path, &metadata_path)?;
    fs::create_dir_all(&dir).map_err(|err| {
        PolyError::raw_source(
            "POLY_LARGE_CORPUS_WS_EDGE_DIR_CREATE_FAILED",
            format!("create WebSocket edge dir {}: {err}", dir.display()),
        )
    })?;
    let request_bytes = persist_websocket_request(&request_path, plan)?;
    let capture = run_websocket_capture(plan, request, clock)?;
    let body_bytes = write_websocket_body(&body_path, plan, &capture)?;
    let body_value = serde_json::from_slice::<Value>(&body_bytes).map_err(|err| {
        PolyError::raw_source(
            "POLY_LARGE_CORPUS_WS_EDGE_BODY_READBACK_DECODE_FAILED",
            format!(
                "decode just-written WebSocket edge body {}: {err}",
                body_path.display()
            ),
        )
    })?;
    let no_payload_window = capture.data_event_count == 0;
    let mut edge = LargeCorpusEdgeCase {
        name: plan.dataset.clone(),
        method: "WEBSOCKET".to_string(),
        url: plan.url.clone(),
        request_path: Some(request_path.display().to_string()),
        request_body_bytes: request_bytes.len() as u64,
        request_body_sha256: Some(sha256_hex(&request_bytes)),
        expected_semantics: plan.expectation.as_str().to_string(),
        status_code: capture.status_code,
        expectation_met: expectation_met(plan, &capture),
        record_count: record_count(&body_value),
        body_path: body_path.display().to_string(),
        metadata_path: metadata_path.display().to_string(),
        body_format: "json".to_string(),
        json_parse_ok: true,
        body_sha256: Some(sha256_hex(&body_bytes)),
        websocket_frame_count: Some(capture.frames.len()),
        websocket_json_frame_count: Some(capture.json_frame_count),
        websocket_event_types: capture.event_types.clone(),
        no_payload_window,
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

fn run_websocket_capture(
    plan: &WebSocketCapturePlan,
    request: &LargeCorpusRequest,
    clock: &dyn Clock,
) -> Result<WebSocketCaptureOutput> {
    let wait = Duration::from_secs(request.timeout_secs.clamp(1, plan.max_wait_secs));
    let mut output = WebSocketCaptureOutput::failed("POLY_LARGE_CORPUS_WS_CONNECT_FAILED", None);
    let (mut socket, response) = match connect_with_timeout(&plan.url, wait) {
        Ok(connected) => connected,
        Err(err) => {
            output.error_message = Some(err.message());
            return Ok(output);
        }
    };
    output.status_code = Some(response.status().as_u16());
    output.handshake_success = true;
    output.error_code = None;
    output.error_message = None;
    set_socket_read_timeout(&mut socket, Duration::from_secs(1))?;
    send_outbound_messages(&mut socket, plan, &mut output)?;

    let deadline = Instant::now() + wait;
    let mut next_heartbeat = Instant::now() + Duration::from_secs(plan.heartbeat_interval_secs);
    while output.frames.len() < plan.max_frames && Instant::now() < deadline {
        if let Some(message) = plan.heartbeat_message
            && plan.heartbeat_interval_secs > 0
            && Instant::now() >= next_heartbeat
        {
            send_text(&mut socket, message)?;
            output.outbound_messages.push(message.to_string());
            next_heartbeat += Duration::from_secs(plan.heartbeat_interval_secs);
        }
        let message = match socket.read() {
            Ok(message) => message,
            Err(tungstenite::Error::Io(err)) if is_timeout(&err) => {
                output.timeout_seen = true;
                continue;
            }
            Err(tungstenite::Error::ConnectionClosed | tungstenite::Error::AlreadyClosed) => break,
            Err(err) => {
                output.error_code = Some("POLY_LARGE_CORPUS_WS_READ_FAILED".to_string());
                output.error_message = Some(err.to_string());
                break;
            }
        };
        if let Err(err) = record_message(&mut socket, plan, message, &mut output, request, clock) {
            output.error_code = Some(err.code().to_string());
            output.error_message = Some(err.message());
            break;
        }
        if expectation_met(plan, &output) {
            break;
        }
    }
    output.event_types = output.event_type_set.iter().cloned().collect();
    Ok(output)
}

fn send_outbound_messages(
    socket: &mut WebSocket<MaybeTlsStream<TcpStream>>,
    plan: &WebSocketCapturePlan,
    output: &mut WebSocketCaptureOutput,
) -> Result<()> {
    for message in &plan.outbound_messages {
        send_text(socket, message)?;
        output.outbound_messages.push(message.clone());
    }
    if plan.send_initial_heartbeat
        && let Some(message) = plan.heartbeat_message
    {
        send_text(socket, message)?;
        output.outbound_messages.push(message.to_string());
    }
    Ok(())
}

fn record_message(
    socket: &mut WebSocket<MaybeTlsStream<TcpStream>>,
    plan: &WebSocketCapturePlan,
    message: Message,
    output: &mut WebSocketCaptureOutput,
    request: &LargeCorpusRequest,
    clock: &dyn Clock,
) -> Result<()> {
    match message {
        Message::Text(text) => {
            let text = text.to_string();
            if text == "ping" && plan.respond_to_text_ping {
                send_text(socket, "pong")?;
                output.outbound_messages.push("pong".to_string());
            }
            let parsed = serde_json::from_str::<Value>(&text);
            let (event_type, json_parse_ok) = match parsed {
                Ok(value) => {
                    let (event_type, _) = match plan.shape {
                        WebSocketShape::Market => json_shape(&value),
                        WebSocketShape::Public => public_json_shape(&value),
                    };
                    output.json_values.push(value);
                    output.json_frame_count += 1;
                    (event_type, true)
                }
                Err(_) => (None, false),
            };
            if let Some(event_type) = &event_type {
                output.event_type_set.insert(event_type.clone());
                if event_type != "rtds_error" {
                    output.data_event_count += 1;
                }
            }
            let raw = text.as_bytes().to_vec();
            push_frame(
                output,
                FrameInput {
                    opcode: "text",
                    body: &raw,
                    raw_text: Some(text),
                    json_parse_ok,
                    event_type,
                    error_code: parse_error_code(json_parse_ok),
                    max_body_bytes: request.max_body_bytes,
                },
                clock,
            )?;
        }
        Message::Ping(payload) => {
            socket.send(Message::Pong(payload.clone())).map_err(|err| {
                PolyError::raw_source(
                    "POLY_LARGE_CORPUS_WS_PONG_SEND_FAILED",
                    format!("respond to WebSocket ping: {err}"),
                )
            })?;
            push_frame(
                output,
                FrameInput {
                    opcode: "ping",
                    body: payload.as_ref(),
                    raw_text: None,
                    json_parse_ok: false,
                    event_type: None,
                    error_code: None,
                    max_body_bytes: request.max_body_bytes,
                },
                clock,
            )?;
        }
        Message::Pong(payload) => push_frame(
            output,
            FrameInput {
                opcode: "pong",
                body: payload.as_ref(),
                raw_text: None,
                json_parse_ok: false,
                event_type: None,
                error_code: None,
                max_body_bytes: request.max_body_bytes,
            },
            clock,
        )?,
        Message::Binary(payload) => push_frame(
            output,
            FrameInput {
                opcode: "binary",
                body: payload.as_ref(),
                raw_text: None,
                json_parse_ok: false,
                event_type: None,
                error_code: Some("POLY_LARGE_CORPUS_WS_BINARY_FRAME".to_string()),
                max_body_bytes: request.max_body_bytes,
            },
            clock,
        )?,
        Message::Close(_) => push_frame(
            output,
            FrameInput {
                opcode: "close",
                body: &[],
                raw_text: None,
                json_parse_ok: false,
                event_type: None,
                error_code: None,
                max_body_bytes: request.max_body_bytes,
            },
            clock,
        )?,
        Message::Frame(_) => push_frame(
            output,
            FrameInput {
                opcode: "frame",
                body: &[],
                raw_text: None,
                json_parse_ok: false,
                event_type: None,
                error_code: Some("POLY_LARGE_CORPUS_WS_UNEXPECTED_RAW_FRAME".to_string()),
                max_body_bytes: request.max_body_bytes,
            },
            clock,
        )?,
    }
    Ok(())
}

fn push_frame(
    output: &mut WebSocketCaptureOutput,
    input: FrameInput<'_>,
    clock: &dyn Clock,
) -> Result<()> {
    let next_total = output.raw_frame_bytes + input.body.len();
    if next_total > input.max_body_bytes {
        return Err(PolyError::raw_source(
            "POLY_LARGE_CORPUS_WS_BODY_LIMIT_EXCEEDED",
            format!(
                "WebSocket frame corpus exceeded {} bytes",
                input.max_body_bytes
            ),
        ));
    }
    output.raw_frame_bytes = next_total;
    output.frames.push(LargeWebSocketFrame {
        direction: "inbound".to_string(),
        opcode: input.opcode.to_string(),
        received_at_unix_ms: now_unix_ms(clock),
        body_bytes: input.body.len() as u64,
        body_sha256: (!input.body.is_empty()).then(|| sha256_hex(input.body)),
        raw_text: input.raw_text,
        json_parse_ok: input.json_parse_ok,
        event_type: input.event_type,
        error_code: input.error_code,
    });
    Ok(())
}

struct FrameInput<'a> {
    opcode: &'static str,
    body: &'a [u8],
    raw_text: Option<String>,
    json_parse_ok: bool,
    event_type: Option<String>,
    error_code: Option<String>,
    max_body_bytes: usize,
}
