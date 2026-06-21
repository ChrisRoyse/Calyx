//! Loopback-only mTLS MCP-over-socket transport for `calyxd` (PH65 · T05).
//!
//! This module is *transport*, not protocol: it owns the length-prefixed
//! framing on a loopback TCP socket, requires a verified client certificate,
//! then hands each decoded JSON-RPC request to a shared [`calyx_mcp::McpServer`]
//! with an [`calyx_core::AuthN::MtlsToken`] identity.
//!
//! ## Why std threads, not tokio
//! The whole workspace is synchronous — there is no tokio dependency anywhere.
//! The existing `/metrics` [`crate::server::MetricsServer`] is already a
//! thread-per-connection `std::net::TcpListener`; this transport follows the
//! same grain rather than dragging in an async runtime for one accept loop.
//!
//! ## Wire format
//! After the TLS handshake, each message is a 4-byte big-endian `u32` length
//! prefix followed by exactly that many bytes of UTF-8 JSON. A length over
//! [`MAX_FRAME_BYTES`] is refused before allocation and closes the connection.
//!
//! ## Fail-closed posture
//! - Non-loopback bind → [`DaemonError::bind_failed`] (`CALYX_DAEMON_BIND_FAILED`);
//!   the server never starts.
//! - Missing/invalid `mcp_mtls` config or absent client cert →
//!   [`CALYX_TLS_CONFIG_INVALID`] / TLS handshake failure before dispatch.
//! - Oversized/garbage frame prefix → [`CALYX_DAEMON_FRAME_INVALID`], connection
//!   closed (the byte stream can no longer be trusted).
//! - Malformed JSON inside a valid frame → a JSON-RPC error response is written
//!   back (carrying the `CALYX_MCP_JSONRPC_INVALID` code) and the connection
//!   stays open for the next frame.
//! - A panicking connection handler → caught, logged as
//!   [`CALYX_DAEMON_CONN_PANIC`], and the accept loop survives.

use std::io::{ErrorKind, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use calyx_core::{AuthN, MtlsConfig};
use calyx_mcp::{JsonRpcError, JsonRpcResponse, McpServer, decode_jsonrpc_request};
use rustls::ServerConfig;

use crate::config::CalyxConfig;
use crate::error::DaemonError;

mod tls;

/// Daemon-local code for an unrecoverable framing error (oversized length prefix
/// or a truncated/garbage frame). Kept MCP-local rather than widening the closed
/// `calyx-core` catalog, mirroring `calyx-mcp`'s own local codes.
pub const CALYX_DAEMON_FRAME_INVALID: &str = "CALYX_DAEMON_FRAME_INVALID";

/// Daemon-local code logged when a connection handler panics. The accept loop
/// isolates it; the connection is dropped, the server keeps serving.
pub const CALYX_DAEMON_CONN_PANIC: &str = "CALYX_DAEMON_CONN_PANIC";

/// Hard ceiling on a single inbound frame, in bytes. A length prefix larger than
/// this is refused before allocating — the mandatory "sanity check" that stops a
/// hostile or buggy peer from requesting a multi-gigabyte buffer (OOM/DoS).
pub const MAX_FRAME_BYTES: u32 = 4 * 1024 * 1024;

/// Per-connection read/write timeout; a stuck peer cannot pin a handler thread.
const IO_TIMEOUT: Duration = Duration::from_secs(5);

/// On shutdown, in-flight connections are given this long to drain before the
/// accept thread returns and the process proceeds to exit.
const DRAIN_TIMEOUT: Duration = Duration::from_secs(5);

/// One decoded frame, or a clean end-of-stream at a frame boundary.
#[derive(Debug)]
enum FrameRead {
    /// A complete message payload (the bytes between the length prefix and the
    /// next frame).
    Payload(Vec<u8>),
    /// The peer closed the connection cleanly at a frame boundary.
    Eof,
}

/// Loopback MCP dispatch server.
///
/// Construct with [`CalyxMcpServer::bind`] (or [`CalyxMcpServer::from_config`]),
/// take a [`ShutdownHandle`] *before* calling [`CalyxMcpServer::run`] (which
/// consumes the server and blocks on the accept loop), then signal shutdown from
/// any thread via the handle.
pub struct CalyxMcpServer {
    listener: TcpListener,
    dispatcher: Arc<McpServer>,
    tls_config: Arc<ServerConfig>,
    shutdown: Arc<AtomicBool>,
    active: Arc<AtomicUsize>,
}

impl CalyxMcpServer {
    /// Binds `addr`, refusing any non-loopback IP before touching the OS so a
    /// misconfiguration can never expose the daemon off-host (A16/A17). Cloudflare
    /// Tunnel + Caddy are the sole external ingress.
    pub fn bind(
        addr: SocketAddr,
        dispatcher: Arc<McpServer>,
        mtls: MtlsConfig,
    ) -> Result<Self, DaemonError> {
        if !addr.ip().is_loopback() {
            return Err(DaemonError::bind_failed(format!(
                "refused non-loopback bind address {addr}; calyxd MCP serves loopback only"
            )));
        }
        let tls_config = tls::build_server_config(&mtls)?;
        let listener = TcpListener::bind(addr)
            .map_err(|error| DaemonError::bind_failed(format!("bind {addr}: {error}")))?;
        Ok(Self {
            listener,
            dispatcher,
            tls_config,
            shutdown: Arc::new(AtomicBool::new(false)),
            active: Arc::new(AtomicUsize::new(0)),
        })
    }

    /// Binds the configured `cfg.bind_addr` (already validated loopback at config
    /// parse — this re-asserts it at the OS boundary per the card).
    pub fn from_config(cfg: &CalyxConfig, dispatcher: Arc<McpServer>) -> Result<Self, DaemonError> {
        let mtls = cfg.mcp_mtls.clone().ok_or_else(|| {
            DaemonError::tls_config_invalid(
                "mcp_mtls is required before calyxd MCP can accept network connections",
            )
        })?;
        Self::bind(cfg.bind_addr, dispatcher, mtls)
    }

    /// The actually-bound address (resolves an OS-assigned port when `:0`).
    pub fn local_addr(&self) -> Result<SocketAddr, DaemonError> {
        self.listener
            .local_addr()
            .map_err(|error| DaemonError::bind_failed(format!("local_addr: {error}")))
    }

    /// A cloneable handle to stop the server and observe live connection count.
    /// Obtain it before [`run`](Self::run), which consumes `self`.
    pub fn shutdown_handle(&self) -> Result<ShutdownHandle, DaemonError> {
        Ok(ShutdownHandle {
            shutdown: Arc::clone(&self.shutdown),
            active: Arc::clone(&self.active),
            addr: self.local_addr()?,
        })
    }

    /// Accept loop. Each connection is served on its own thread, with panics
    /// isolated so one bad client cannot crash the daemon. Blocks until a
    /// [`ShutdownHandle::shutdown`] fires, then drains in-flight connections for
    /// up to [`DRAIN_TIMEOUT`] before returning.
    pub fn run(self) -> Result<(), DaemonError> {
        loop {
            match self.listener.accept() {
                Ok((stream, peer)) => {
                    // The accept may have been woken by the shutdown self-connect;
                    // do not serve that throwaway connection.
                    if self.shutdown.load(Ordering::SeqCst) {
                        break;
                    }
                    self.active.fetch_add(1, Ordering::SeqCst);
                    let dispatcher = Arc::clone(&self.dispatcher);
                    let tls_config = Arc::clone(&self.tls_config);
                    let active = Arc::clone(&self.active);
                    std::thread::spawn(move || {
                        let outcome = catch_unwind(AssertUnwindSafe(|| {
                            tls::serve_connection(stream, &dispatcher, tls_config)
                        }));
                        active.fetch_sub(1, Ordering::SeqCst);
                        match outcome {
                            Ok(Ok(())) => {}
                            Ok(Err(detail)) => {
                                eprintln!("calyxd: mcp connection from {peer}: {detail}");
                            }
                            Err(_panic) => {
                                eprintln!(
                                    "calyxd: {CALYX_DAEMON_CONN_PANIC}: mcp connection from \
                                     {peer} panicked; connection dropped, server continues"
                                );
                            }
                        }
                    });
                }
                Err(error) => {
                    if self.shutdown.load(Ordering::SeqCst) {
                        break;
                    }
                    eprintln!("calyxd: accept on mcp listener failed: {error}");
                }
            }
        }

        let deadline = Instant::now() + DRAIN_TIMEOUT;
        while self.active.load(Ordering::SeqCst) > 0 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        Ok(())
    }
}

/// Stops a running [`CalyxMcpServer`] and reports live connection count.
///
/// [`shutdown`](Self::shutdown) sets the stop flag, then opens a throwaway
/// loopback connection to the bound address to wake the blocked `accept()` — the
/// standard std-thread idiom for unblocking a synchronous listener without
/// busy-polling.
#[derive(Clone)]
pub struct ShutdownHandle {
    shutdown: Arc<AtomicBool>,
    active: Arc<AtomicUsize>,
    addr: SocketAddr,
}

impl ShutdownHandle {
    /// Signals the accept loop to stop and wakes it so `run` returns promptly.
    pub fn shutdown(&self) {
        self.shutdown.store(true, Ordering::SeqCst);
        // Wake the blocked accept(); the loop sees the flag and breaks. A failed
        // connect is harmless — the listener may already be draining/closed.
        let _ = TcpStream::connect(self.addr);
    }

    /// Number of connection handlers currently in flight (0 once all drain).
    pub fn active_connections(&self) -> usize {
        self.active.load(Ordering::SeqCst)
    }
}

/// Serves length-prefixed JSON-RPC requests on an already-authenticated stream
/// until EOF or an unrecoverable framing error. Each decoded request is
/// dispatched through the shared [`McpServer`] with the caller identity derived
/// by the transport; notifications (no `id`) get no reply, per JSON-RPC 2.0.
fn serve_stream(
    stream: &mut impl ReadWrite,
    dispatcher: &McpServer,
    authn: Option<&AuthN>,
) -> Result<(), String> {
    loop {
        let payload = match read_frame(stream)? {
            FrameRead::Payload(bytes) => bytes,
            FrameRead::Eof => return Ok(()),
        };

        match decode_jsonrpc_request(&payload) {
            Ok(request) => {
                let is_notification = request.id.is_none();
                let response = dispatcher.dispatch_with_authn(request, authn);
                if is_notification {
                    continue;
                }
                write_response(stream, &response)?;
            }
            Err(calyx) => {
                // Malformed JSON in an otherwise valid frame is a per-message
                // error, not a stream error: answer with a structured JSON-RPC
                // error (id unknown → null) and keep serving the next frame.
                let response = JsonRpcResponse::error(None, JsonRpcError::from_calyx(&calyx));
                write_response(stream, &response)?;
            }
        }
    }
}

trait ReadWrite: Read + Write {}

impl<T: Read + Write> ReadWrite for T {}

/// Serializes `response` and writes it as one length-prefixed frame.
fn write_response(stream: &mut impl Write, response: &JsonRpcResponse) -> Result<(), String> {
    let body = serde_json::to_vec(response)
        .map_err(|error| format!("serialize JSON-RPC response: {error}"))?;
    write_frame(stream, &body)
}

/// Reads one length-prefixed frame. Returns [`FrameRead::Eof`] only when the peer
/// closes exactly at a frame boundary (no partial prefix). A length over
/// [`MAX_FRAME_BYTES`] or a truncated frame is an `Err` that closes the stream.
fn read_frame(reader: &mut impl Read) -> Result<FrameRead, String> {
    let mut len_prefix = [0_u8; 4];
    match read_full_or_eof(reader, &mut len_prefix)? {
        ReadState::Eof => return Ok(FrameRead::Eof),
        ReadState::Filled => {}
    }
    let len = u32::from_be_bytes(len_prefix);
    if len == 0 {
        return Err(format!(
            "{CALYX_DAEMON_FRAME_INVALID}: zero-length frame is not a valid MCP message"
        ));
    }
    if len > MAX_FRAME_BYTES {
        return Err(format!(
            "{CALYX_DAEMON_FRAME_INVALID}: frame length {len} exceeds maximum {MAX_FRAME_BYTES} \
             bytes; refusing allocation and closing connection"
        ));
    }
    let mut payload = vec![0_u8; len as usize];
    reader
        .read_exact(&mut payload)
        .map_err(|error| format!("read {len}-byte frame body: {error}"))?;
    Ok(FrameRead::Payload(payload))
}

/// Writes a 4-byte big-endian length prefix followed by `payload`.
fn write_frame(writer: &mut impl Write, payload: &[u8]) -> Result<(), String> {
    let len = u32::try_from(payload.len()).map_err(|_| {
        format!(
            "response of {} bytes exceeds u32 frame prefix",
            payload.len()
        )
    })?;
    writer
        .write_all(&len.to_be_bytes())
        .map_err(|error| format!("write frame prefix: {error}"))?;
    writer
        .write_all(payload)
        .map_err(|error| format!("write frame body: {error}"))?;
    writer
        .flush()
        .map_err(|error| format!("flush frame: {error}"))
}

/// Outcome of trying to fill a fixed buffer: fully read, or a clean EOF before
/// any byte arrived.
enum ReadState {
    Filled,
    Eof,
}

/// Fills `buf` completely, transparently retrying short/`Interrupted` reads. A
/// zero-byte read on the *first* attempt is a clean EOF; a zero-byte read after
/// partial data is a truncated frame (error), never silently accepted.
fn read_full_or_eof(reader: &mut impl Read, buf: &mut [u8]) -> Result<ReadState, String> {
    let mut filled = 0;
    while filled < buf.len() {
        match reader.read(&mut buf[filled..]) {
            Ok(0) => {
                if filled == 0 {
                    return Ok(ReadState::Eof);
                }
                return Err(format!(
                    "{CALYX_DAEMON_FRAME_INVALID}: truncated frame prefix ({filled} of {} bytes)",
                    buf.len()
                ));
            }
            Ok(n) => filled += n,
            Err(error) if error.kind() == ErrorKind::Interrupted => continue,
            Err(error) => return Err(format!("read frame prefix: {error}")),
        }
    }
    Ok(ReadState::Filled)
}

#[cfg(test)]
mod tests;
