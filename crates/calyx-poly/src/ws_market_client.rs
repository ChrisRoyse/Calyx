use std::collections::BTreeSet;
use std::net::TcpStream;
use std::time::{Duration, Instant};

use calyx_core::Clock;
use serde::{Deserialize, Serialize};
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{Message, WebSocket};

use crate::Result;
use crate::raw_source_support::{now_unix_ms, sha256_hex};
use crate::raw_websocket_support::{
    connect_with_timeout, is_timeout, send_text, set_socket_read_timeout,
};
use crate::ws_market_parse::parse_market_ws_text;
use crate::ws_market_types::{
    ERR_WS_MARKET_BODY_LIMIT, ERR_WS_MARKET_CONNECT, ERR_WS_MARKET_EVENT_INVALID,
    ERR_WS_MARKET_NO_PAYLOAD_WINDOW, ERR_WS_MARKET_READ, ERR_WS_MARKET_READBACK_MISMATCH,
    ERR_WS_MARKET_REQUEST_INVALID, ERR_WS_MARKET_SEND, ERR_WS_MARKET_SESSION_INCOMPLETE,
    MarketWsClientConfig, MarketWsControlMessage, MarketWsParsedEvent, MarketWsSubscription,
    validate_market_ws_config, validate_subscription, ws_market_error,
};

pub struct MarketWsClient {
    config: MarketWsClientConfig,
}

impl MarketWsClient {
    pub fn new(config: MarketWsClientConfig) -> Result<Self> {
        validate_market_ws_config(&config)?;
        Ok(Self { config })
    }

    pub fn config(&self) -> &MarketWsClientConfig {
        &self.config
    }

    pub fn capture_window(
        &self,
        subscription: &MarketWsSubscription,
        session_index: usize,
        clock: &dyn Clock,
    ) -> Result<MarketWsCaptureSession> {
        validate_subscription(subscription)?;
        let wait = Duration::from_secs(self.config.timeout_secs);
        let mut session = MarketWsCaptureSession::new(session_index);
        let (mut socket, response) =
            connect_with_timeout(&self.config.url, wait).map_err(|err| {
                ws_market_error(
                    ERR_WS_MARKET_CONNECT,
                    format!(
                        "connect market WebSocket {}: {}",
                        self.config.url,
                        err.message()
                    ),
                )
            })?;
        session.status_code = Some(response.status().as_u16());
        session.handshake_success = true;
        set_socket_read_timeout(&mut socket, Duration::from_secs(1))?;
        self.send_outbound(&mut socket, subscription, &mut session)?;

        let deadline = Instant::now() + wait;
        let mut next_heartbeat = Instant::now() + Duration::from_secs(self.config.heartbeat_secs);
        while session.frames.len() < self.config.max_frames && Instant::now() < deadline {
            if self.config.heartbeat_secs > 0 && Instant::now() >= next_heartbeat {
                self.send_market_text(&mut socket, "PING", &mut session)?;
                next_heartbeat += Duration::from_secs(self.config.heartbeat_secs);
            }
            let message = match socket.read() {
                Ok(message) => message,
                Err(tungstenite::Error::Io(err)) if is_timeout(&err) => {
                    session.timeout_seen = true;
                    continue;
                }
                Err(tungstenite::Error::ConnectionClosed | tungstenite::Error::AlreadyClosed) => {
                    break;
                }
                Err(err) => {
                    session.error_code = Some(ERR_WS_MARKET_READ.to_string());
                    session.error_message = Some(err.to_string());
                    break;
                }
            };
            self.record_message(&mut socket, &mut session, message, clock)?;
            if self.session_complete(&session) || session.error_code.is_some() {
                break;
            }
        }
        finalize_session(&mut session)?;
        Ok(session)
    }

    pub fn capture_reconnect(
        &self,
        subscription: &MarketWsSubscription,
        session_count: usize,
        clock: &dyn Clock,
    ) -> Result<Vec<MarketWsCaptureSession>> {
        if session_count == 0 {
            return Err(ws_market_error(
                ERR_WS_MARKET_REQUEST_INVALID,
                "market WebSocket reconnect capture requires at least one session",
            ));
        }
        let mut sessions = Vec::with_capacity(session_count);
        for session_index in 0..session_count {
            let session = self.capture_window(subscription, session_index, clock)?;
            require_market_ws_session_data(&session, &self.config)?;
            sessions.push(session);
        }
        Ok(sessions)
    }

    fn session_complete(&self, session: &MarketWsCaptureSession) -> bool {
        session.data_event_count >= self.config.min_data_events
            && (!self.config.require_pong || session.pong_received)
    }

    fn send_outbound(
        &self,
        socket: &mut WebSocket<MaybeTlsStream<TcpStream>>,
        subscription: &MarketWsSubscription,
        session: &mut MarketWsCaptureSession,
    ) -> Result<()> {
        self.send_market_text(socket, &subscription.to_wire_text()?, session)?;
        if self.config.require_pong || self.config.heartbeat_secs > 0 {
            self.send_market_text(socket, "PING", session)?;
        }
        Ok(())
    }

    fn send_market_text(
        &self,
        socket: &mut WebSocket<MaybeTlsStream<TcpStream>>,
        text: &str,
        session: &mut MarketWsCaptureSession,
    ) -> Result<()> {
        send_text(socket, text).map_err(|err| {
            ws_market_error(
                ERR_WS_MARKET_SEND,
                format!("send market WebSocket message: {}", err.message()),
            )
        })?;
        session.outbound_messages.push(text.to_string());
        Ok(())
    }

    fn record_message(
        &self,
        socket: &mut WebSocket<MaybeTlsStream<TcpStream>>,
        session: &mut MarketWsCaptureSession,
        message: Message,
        clock: &dyn Clock,
    ) -> Result<()> {
        match message {
            Message::Text(text) => self.record_text(session, text.to_string(), clock),
            Message::Ping(payload) => {
                socket.send(Message::Pong(payload.clone())).map_err(|err| {
                    ws_market_error(
                        ERR_WS_MARKET_SEND,
                        format!("respond to server WebSocket ping: {err}"),
                    )
                })?;
                push_frame(
                    session,
                    FrameInput {
                        opcode: "ping",
                        body: payload.as_ref(),
                        raw_text: None,
                        json_parse_ok: false,
                        control: None,
                        events: Vec::new(),
                        error_code: None,
                        max_body_bytes: self.config.max_body_bytes,
                    },
                    clock,
                )
            }
            Message::Pong(payload) => {
                session.pong_received = true;
                push_frame(
                    session,
                    FrameInput {
                        opcode: "pong",
                        body: payload.as_ref(),
                        raw_text: None,
                        json_parse_ok: false,
                        control: Some(MarketWsControlMessage::Pong),
                        events: Vec::new(),
                        error_code: None,
                        max_body_bytes: self.config.max_body_bytes,
                    },
                    clock,
                )
            }
            Message::Binary(payload) => {
                let code = ERR_WS_MARKET_EVENT_INVALID.to_string();
                session.error_code = Some(code.clone());
                session.error_message = Some("market WebSocket emitted binary frame".to_string());
                push_frame(
                    session,
                    FrameInput {
                        opcode: "binary",
                        body: payload.as_ref(),
                        raw_text: None,
                        json_parse_ok: false,
                        control: None,
                        events: Vec::new(),
                        error_code: Some(code),
                        max_body_bytes: self.config.max_body_bytes,
                    },
                    clock,
                )
            }
            Message::Close(_) => push_frame(
                session,
                FrameInput {
                    opcode: "close",
                    body: &[],
                    raw_text: None,
                    json_parse_ok: false,
                    control: None,
                    events: Vec::new(),
                    error_code: None,
                    max_body_bytes: self.config.max_body_bytes,
                },
                clock,
            ),
            Message::Frame(_) => {
                let code = ERR_WS_MARKET_EVENT_INVALID.to_string();
                session.error_code = Some(code.clone());
                session.error_message =
                    Some("market WebSocket emitted unexpected raw frame".to_string());
                push_frame(
                    session,
                    FrameInput {
                        opcode: "frame",
                        body: &[],
                        raw_text: None,
                        json_parse_ok: false,
                        control: None,
                        events: Vec::new(),
                        error_code: Some(code),
                        max_body_bytes: self.config.max_body_bytes,
                    },
                    clock,
                )
            }
        }
    }

    fn record_text(
        &self,
        session: &mut MarketWsCaptureSession,
        text: String,
        clock: &dyn Clock,
    ) -> Result<()> {
        match parse_market_ws_text(&text) {
            Ok(envelope) => {
                let body = text.as_bytes().to_vec();
                if envelope.control == Some(MarketWsControlMessage::Pong) {
                    session.pong_received = true;
                }
                for event in &envelope.events {
                    if event.is_market_data() {
                        session.data_event_count += 1;
                    }
                    if event.is_lifecycle() {
                        session.lifecycle_event_count += 1;
                    }
                }
                let json_parse_ok = envelope.control.is_none();
                push_frame(
                    session,
                    FrameInput {
                        opcode: "text",
                        body: &body,
                        raw_text: Some(text),
                        json_parse_ok,
                        control: envelope.control,
                        events: envelope.events,
                        error_code: None,
                        max_body_bytes: self.config.max_body_bytes,
                    },
                    clock,
                )
            }
            Err(err) => {
                let body = text.as_bytes().to_vec();
                let code = err.code().to_string();
                session.error_code = Some(code.clone());
                session.error_message = Some(err.message());
                push_frame(
                    session,
                    FrameInput {
                        opcode: "text",
                        body: &body,
                        raw_text: Some(text),
                        json_parse_ok: false,
                        control: None,
                        events: Vec::new(),
                        error_code: Some(code),
                        max_body_bytes: self.config.max_body_bytes,
                    },
                    clock,
                )
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MarketWsFrameRecord {
    pub direction: String,
    pub opcode: String,
    pub received_at_unix_ms: u128,
    pub body_bytes: u64,
    pub body_sha256: Option<String>,
    pub raw_text: Option<String>,
    pub json_parse_ok: bool,
    pub control: Option<MarketWsControlMessage>,
    pub event_types: Vec<String>,
    pub events: Vec<MarketWsParsedEvent>,
    pub error_code: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MarketWsCaptureSession {
    pub session_index: usize,
    pub status_code: Option<u16>,
    pub handshake_success: bool,
    pub outbound_messages: Vec<String>,
    pub frames: Vec<MarketWsFrameRecord>,
    pub event_types: Vec<String>,
    pub data_event_count: usize,
    pub lifecycle_event_count: usize,
    pub pong_received: bool,
    pub no_payload_window: bool,
    pub timeout_seen: bool,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
    pub payload_bytes: u64,
    pub body_bytes: u64,
    pub body_sha256: Option<String>,
}

impl MarketWsCaptureSession {
    pub fn new(session_index: usize) -> Self {
        Self {
            session_index,
            status_code: None,
            handshake_success: false,
            outbound_messages: Vec::new(),
            frames: Vec::new(),
            event_types: Vec::new(),
            data_event_count: 0,
            lifecycle_event_count: 0,
            pong_received: false,
            no_payload_window: true,
            timeout_seen: false,
            error_code: None,
            error_message: None,
            payload_bytes: 0,
            body_bytes: 0,
            body_sha256: None,
        }
    }
}

pub fn require_market_ws_session_data(
    session: &MarketWsCaptureSession,
    config: &MarketWsClientConfig,
) -> Result<()> {
    if !session.handshake_success {
        return Err(ws_market_error(
            ERR_WS_MARKET_SESSION_INCOMPLETE,
            "market WebSocket handshake did not complete",
        ));
    }
    if let Some(code) = &session.error_code {
        return Err(ws_market_error(
            code,
            session
                .error_message
                .clone()
                .unwrap_or_else(|| "market WebSocket session failed".to_string()),
        ));
    }
    if session.data_event_count < config.min_data_events {
        return Err(ws_market_error(
            ERR_WS_MARKET_NO_PAYLOAD_WINDOW,
            format!(
                "market WebSocket produced {} data events, required {}",
                session.data_event_count, config.min_data_events
            ),
        ));
    }
    if config.require_pong && !session.pong_received {
        return Err(ws_market_error(
            ERR_WS_MARKET_SESSION_INCOMPLETE,
            "market WebSocket heartbeat PONG was not observed",
        ));
    }
    Ok(())
}

fn push_frame(
    session: &mut MarketWsCaptureSession,
    input: FrameInput<'_>,
    clock: &dyn Clock,
) -> Result<()> {
    let next_payload = session.payload_bytes + input.body.len() as u64;
    if next_payload > input.max_body_bytes as u64 {
        return Err(ws_market_error(
            ERR_WS_MARKET_BODY_LIMIT,
            format!(
                "market WebSocket frames exceeded {} bytes",
                input.max_body_bytes
            ),
        ));
    }
    session.payload_bytes = next_payload;
    let event_types = input
        .events
        .iter()
        .map(|event| event.event_type().to_string())
        .collect::<Vec<_>>();
    session.frames.push(MarketWsFrameRecord {
        direction: "inbound".to_string(),
        opcode: input.opcode.to_string(),
        received_at_unix_ms: now_unix_ms(clock),
        body_bytes: input.body.len() as u64,
        body_sha256: (!input.body.is_empty()).then(|| sha256_hex(input.body)),
        raw_text: input.raw_text,
        json_parse_ok: input.json_parse_ok,
        control: input.control,
        event_types,
        events: input.events,
        error_code: input.error_code,
    });
    Ok(())
}

fn finalize_session(session: &mut MarketWsCaptureSession) -> Result<()> {
    let mut event_types = BTreeSet::new();
    for frame in &session.frames {
        event_types.extend(frame.event_types.iter().cloned());
    }
    session.event_types = event_types.into_iter().collect();
    session.no_payload_window = session.data_event_count == 0;
    let bytes = serde_json::to_vec(&session.frames).map_err(|err| {
        ws_market_error(
            ERR_WS_MARKET_READBACK_MISMATCH,
            format!("encode market WebSocket frame summary: {err}"),
        )
    })?;
    session.body_bytes = bytes.len() as u64;
    session.body_sha256 = Some(sha256_hex(&bytes));
    Ok(())
}

struct FrameInput<'a> {
    opcode: &'static str,
    body: &'a [u8],
    raw_text: Option<String>,
    json_parse_ok: bool,
    control: Option<MarketWsControlMessage>,
    events: Vec<MarketWsParsedEvent>,
    error_code: Option<String>,
    max_body_bytes: usize,
}
