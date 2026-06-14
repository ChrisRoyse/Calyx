//! Loopback-only MCP-over-socket transport for `calyxd` (PH65 · T05).
//!
//! This module is *transport*, not protocol: it owns the length-prefixed
//! framing on a loopback TCP socket and hands each decoded JSON-RPC request to a
//! shared [`calyx_mcp::McpServer`], whose dispatch already implements the three
//! MCP methods, per-tool panic isolation, and the `CalyxError` → `-32000`
//! mapping. T05 deliberately does not reimplement any of that, and it does not
//! register production tools (that is PH63's tool groups + T06's `main` wiring).
//!
//! ## Why std threads, not tokio
//! The whole workspace is synchronous — there is no tokio dependency anywhere.
//! The existing `/metrics` [`crate::server::MetricsServer`] is already a
//! thread-per-connection `std::net::TcpListener`; this transport follows the
//! same grain rather than dragging in an async runtime for one accept loop.
//!
//! ## Wire format
//! Each message is a 4-byte big-endian `u32` length prefix followed by exactly
//! that many bytes of UTF-8 JSON (one JSON-RPC request or response). A length
//! over [`MAX_FRAME_BYTES`] is refused before any allocation — the documented
//! DoS guard for length-prefixed framing — and desyncs the stream, so the
//! connection is closed. A clean EOF at a frame boundary is a normal close.
//!
//! ## Fail-closed posture
//! - Non-loopback bind → [`DaemonError::bind_failed`] (`CALYX_DAEMON_BIND_FAILED`);
//!   the server never starts.
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

use calyx_mcp::{JsonRpcError, JsonRpcResponse, McpServer, decode_jsonrpc_request};

use crate::config::CalyxConfig;
use crate::error::DaemonError;

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
    shutdown: Arc<AtomicBool>,
    active: Arc<AtomicUsize>,
}

impl CalyxMcpServer {
    /// Binds `addr`, refusing any non-loopback IP before touching the OS so a
    /// misconfiguration can never expose the daemon off-host (A16/A17). Cloudflare
    /// Tunnel + Caddy are the sole external ingress.
    pub fn bind(addr: SocketAddr, dispatcher: Arc<McpServer>) -> Result<Self, DaemonError> {
        if !addr.ip().is_loopback() {
            return Err(DaemonError::bind_failed(format!(
                "refused non-loopback bind address {addr}; calyxd MCP serves loopback only"
            )));
        }
        let listener = TcpListener::bind(addr)
            .map_err(|error| DaemonError::bind_failed(format!("bind {addr}: {error}")))?;
        Ok(Self {
            listener,
            dispatcher,
            shutdown: Arc::new(AtomicBool::new(false)),
            active: Arc::new(AtomicUsize::new(0)),
        })
    }

    /// Binds the configured `cfg.bind_addr` (already validated loopback at config
    /// parse — this re-asserts it at the OS boundary per the card).
    pub fn from_config(cfg: &CalyxConfig, dispatcher: Arc<McpServer>) -> Result<Self, DaemonError> {
        Self::bind(cfg.bind_addr, dispatcher)
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
                    let active = Arc::clone(&self.active);
                    std::thread::spawn(move || {
                        let outcome = catch_unwind(AssertUnwindSafe(|| {
                            serve_connection(stream, &dispatcher)
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

/// Serves length-prefixed JSON-RPC requests on `stream` until EOF or an
/// unrecoverable framing error. Each decoded request is dispatched through the
/// shared [`McpServer`]; notifications (no `id`) get no reply, per JSON-RPC 2.0.
fn serve_connection(mut stream: TcpStream, dispatcher: &McpServer) -> Result<(), String> {
    stream
        .set_read_timeout(Some(IO_TIMEOUT))
        .map_err(|error| format!("set read timeout: {error}"))?;
    stream
        .set_write_timeout(Some(IO_TIMEOUT))
        .map_err(|error| format!("set write timeout: {error}"))?;

    loop {
        let payload = match read_frame(&mut stream)? {
            FrameRead::Payload(bytes) => bytes,
            FrameRead::Eof => return Ok(()),
        };

        match decode_jsonrpc_request(&payload) {
            Ok(request) => {
                let is_notification = request.id.is_none();
                let response = dispatcher.dispatch(request);
                if is_notification {
                    continue;
                }
                write_response(&mut stream, &response)?;
            }
            Err(calyx) => {
                // Malformed JSON in an otherwise valid frame is a per-message
                // error, not a stream error: answer with a structured JSON-RPC
                // error (id unknown → null) and keep serving the next frame.
                let response = JsonRpcResponse::error(None, JsonRpcError::from_calyx(&calyx));
                write_response(&mut stream, &response)?;
            }
        }
    }
}

/// Serializes `response` and writes it as one length-prefixed frame.
fn write_response(stream: &mut TcpStream, response: &JsonRpcResponse) -> Result<(), String> {
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
mod tests {
    use super::*;
    use std::io::Cursor;

    fn dispatcher() -> Arc<McpServer> {
        Arc::new(McpServer::new())
    }

    #[test]
    fn bind_refuses_non_loopback_address() {
        let Err(error) = CalyxMcpServer::bind("0.0.0.0:7700".parse().unwrap(), dispatcher()) else {
            panic!("non-loopback bind must fail");
        };
        assert_eq!(error.code(), "CALYX_DAEMON_BIND_FAILED");
        assert!(error.to_string().contains("0.0.0.0:7700"));
    }

    #[test]
    fn bind_accepts_ipv4_loopback() {
        let server = CalyxMcpServer::bind("127.0.0.1:0".parse().unwrap(), dispatcher()).unwrap();
        assert!(server.local_addr().unwrap().ip().is_loopback());
    }

    #[test]
    fn bind_accepts_ipv6_loopback() {
        let server = CalyxMcpServer::bind("[::1]:0".parse().unwrap(), dispatcher()).unwrap();
        assert!(server.local_addr().unwrap().ip().is_loopback());
    }

    #[test]
    fn from_config_binds_validated_loopback_addr() {
        let cfg = CalyxConfig::from_toml_str(
            "bind_addr = \"127.0.0.1:0\"\nvault_path = \"/v\"\nvram_budget_mib = 8192\nlog_dir = \"/l\"\n",
        )
        .unwrap();
        let server = CalyxMcpServer::from_config(&cfg, dispatcher()).unwrap();
        assert!(server.local_addr().unwrap().ip().is_loopback());
    }

    #[test]
    fn frame_round_trips_through_codec() {
        // write_frame then read_frame must reconstruct the exact payload bytes.
        let payload = br#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#;
        let mut buf = Vec::new();
        write_frame(&mut buf, payload).unwrap();
        // 4-byte BE prefix == payload length, by hand: 46 bytes.
        assert_eq!(payload.len(), 46);
        assert_eq!(buf[..4], 46_u32.to_be_bytes());
        let mut cursor = Cursor::new(buf);
        match read_frame(&mut cursor).unwrap() {
            FrameRead::Payload(bytes) => assert_eq!(bytes, payload),
            FrameRead::Eof => panic!("expected a payload, got EOF"),
        }
    }

    #[test]
    fn read_frame_reports_clean_eof_at_boundary() {
        let mut cursor = Cursor::new(Vec::new());
        assert!(matches!(read_frame(&mut cursor).unwrap(), FrameRead::Eof));
    }

    #[test]
    fn read_frame_refuses_oversize_length_before_allocating() {
        // Prefix claims MAX+1 bytes with no body present: must be refused on the
        // prefix alone, never attempting the allocation/read.
        let oversize = MAX_FRAME_BYTES + 1;
        let mut cursor = Cursor::new(oversize.to_be_bytes().to_vec());
        let error = read_frame(&mut cursor).unwrap_err();
        assert!(error.contains(CALYX_DAEMON_FRAME_INVALID));
        assert!(error.contains(&oversize.to_string()));
    }

    #[test]
    fn read_frame_rejects_zero_length_frame() {
        let mut cursor = Cursor::new(0_u32.to_be_bytes().to_vec());
        let error = read_frame(&mut cursor).unwrap_err();
        assert!(error.contains(CALYX_DAEMON_FRAME_INVALID));
        assert!(error.contains("zero-length"));
    }

    #[test]
    fn read_frame_rejects_truncated_prefix() {
        // Only 2 of the 4 prefix bytes arrive, then EOF: a truncated frame, not a
        // clean boundary close.
        let mut cursor = Cursor::new(vec![0x00, 0x01]);
        let error = read_frame(&mut cursor).unwrap_err();
        assert!(error.contains(CALYX_DAEMON_FRAME_INVALID));
        assert!(error.contains("truncated"));
    }

    #[test]
    fn read_frame_rejects_truncated_body() {
        // Prefix says 8 bytes, only 3 follow: read_exact must error.
        let mut bytes = 8_u32.to_be_bytes().to_vec();
        bytes.extend_from_slice(b"abc");
        let mut cursor = Cursor::new(bytes);
        let error = read_frame(&mut cursor).unwrap_err();
        assert!(error.contains("frame body"));
    }
}
