use std::fs;
use std::io::ErrorKind;
use std::net::{TcpStream, ToSocketAddrs};
use std::path::PathBuf;
use std::time::Duration;

use serde_json::Value;
use tungstenite::handshake::client::Response;
use tungstenite::http::Uri;
use tungstenite::stream::MaybeTlsStream;
use tungstenite::{Message, WebSocket, client_tls_with_config};

use crate::{PolyError, Result};

pub(crate) fn connect_with_timeout(
    url: &str,
    timeout: Duration,
) -> Result<(WebSocket<MaybeTlsStream<TcpStream>>, Response)> {
    let uri = url.parse::<Uri>().map_err(|err| {
        PolyError::raw_source(
            "POLY_RAW_SOURCE_WS_URL_PARSE_FAILED",
            format!("parse {url}: {err}"),
        )
    })?;
    let host = uri.host().ok_or_else(|| {
        PolyError::raw_source(
            "POLY_RAW_SOURCE_WS_HOST_MISSING",
            format!("missing host in {url}"),
        )
    })?;
    let host = host.trim_start_matches('[').trim_end_matches(']');
    let port = uri.port_u16().unwrap_or(match uri.scheme_str() {
        Some("wss") => 443,
        Some("ws") => 80,
        other => {
            return Err(PolyError::raw_source(
                "POLY_RAW_SOURCE_WS_SCHEME_UNSUPPORTED",
                format!("unsupported WebSocket scheme {other:?} for {url}"),
            ));
        }
    });
    let addresses = (host, port).to_socket_addrs().map_err(|err| {
        PolyError::raw_source(
            "POLY_RAW_SOURCE_WS_DNS_FAILED",
            format!("resolve {host}:{port}: {err}"),
        )
    })?;
    let mut last_error = None;
    for address in addresses {
        match TcpStream::connect_timeout(&address, timeout) {
            Ok(stream) => {
                configure_stream(&stream, address.to_string(), timeout)?;
                return client_tls_with_config(url, stream, None, None).map_err(|err| {
                    PolyError::raw_source(
                        "POLY_RAW_SOURCE_WS_HANDSHAKE_FAILED",
                        format!("handshake {url}: {err}"),
                    )
                });
            }
            Err(err) => last_error = Some(err),
        }
    }
    Err(PolyError::raw_source(
        "POLY_RAW_SOURCE_WS_CONNECT_FAILED",
        format!("connect {url}: {:?}", last_error),
    ))
}

pub(crate) fn send_text(
    socket: &mut WebSocket<MaybeTlsStream<TcpStream>>,
    text: &str,
) -> Result<()> {
    socket.send(Message::Text(text.into())).map_err(|err| {
        PolyError::raw_source(
            "POLY_RAW_SOURCE_WS_SEND_FAILED",
            format!("send WebSocket text message: {err}"),
        )
    })
}

pub(crate) fn set_socket_read_timeout(
    socket: &mut WebSocket<MaybeTlsStream<TcpStream>>,
    timeout: Duration,
) -> Result<()> {
    match socket.get_mut() {
        MaybeTlsStream::Plain(stream) => stream.set_read_timeout(Some(timeout)),
        MaybeTlsStream::Rustls(stream) => stream.sock.set_read_timeout(Some(timeout)),
        _ => {
            return Err(PolyError::raw_source(
                "POLY_RAW_SOURCE_WS_STREAM_UNSUPPORTED",
                "set read timeout on unsupported WebSocket stream transport",
            ));
        }
    }
    .map_err(|err| {
        PolyError::raw_source(
            "POLY_RAW_SOURCE_WS_READ_TIMEOUT_SET_FAILED",
            format!("set WebSocket read timeout: {err}"),
        )
    })
}

pub(crate) fn json_shape(value: &Value) -> (Option<String>, Vec<String>) {
    match value {
        Value::Object(map) => (
            map.get("event_type")
                .and_then(Value::as_str)
                .map(str::to_string),
            map.keys().cloned().collect(),
        ),
        Value::Array(items) => items
            .first()
            .and_then(Value::as_object)
            .map(|map| {
                (
                    map.get("event_type")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    map.keys().cloned().collect(),
                )
            })
            .unwrap_or((None, Vec::new())),
        _ => (None, Vec::new()),
    }
}

pub(crate) fn remove_stale_body(body_path: &PathBuf) -> Result<()> {
    if body_path.exists() {
        fs::remove_file(body_path).map_err(|err| {
            PolyError::raw_source(
                "POLY_RAW_SOURCE_WS_BODY_REMOVE_FAILED",
                format!("remove stale WebSocket body {}: {err}", body_path.display()),
            )
        })?;
    }
    Ok(())
}

pub(crate) fn is_timeout(err: &std::io::Error) -> bool {
    matches!(err.kind(), ErrorKind::TimedOut | ErrorKind::WouldBlock)
}

fn configure_stream(stream: &TcpStream, address: String, timeout: Duration) -> Result<()> {
    stream.set_nodelay(true).map_err(|err| {
        PolyError::raw_source(
            "POLY_RAW_SOURCE_WS_NODELAY_FAILED",
            format!("set nodelay for {address}: {err}"),
        )
    })?;
    stream.set_read_timeout(Some(timeout)).map_err(|err| {
        PolyError::raw_source(
            "POLY_RAW_SOURCE_WS_READ_TIMEOUT_SET_FAILED",
            format!("set read timeout for {address}: {err}"),
        )
    })?;
    stream.set_write_timeout(Some(timeout)).map_err(|err| {
        PolyError::raw_source(
            "POLY_RAW_SOURCE_WS_WRITE_TIMEOUT_SET_FAILED",
            format!("set write timeout for {address}: {err}"),
        )
    })
}
