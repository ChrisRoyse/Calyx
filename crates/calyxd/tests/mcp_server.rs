//! Integration FSV for the PH65 · T05 loopback MCP dispatch transport.
//!
//! These tests drive the *real* server over a *real* loopback TCP socket: bind
//! `127.0.0.1:0`, run the accept loop on a thread, complete an mTLS handshake,
//! and exchange length-prefixed JSON-RPC frames. The dispatcher is a genuine
//! `calyx_mcp::McpServer`; the registered [`AdderTool`] performs real arithmetic
//! (`a + b`) so round-trips assert on hand-computed bytes — no mocks.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use calyx_core::{CalyxError, MtlsConfig, TlsConfig};
use calyx_mcp::{McpServer, Tool, ToolDef, ToolResult};
use calyxd::mcp_server::{CalyxMcpServer, ShutdownHandle};
use rcgen::{CertifiedKey, generate_simple_self_signed};
use rustls::{ClientConfig, ClientConnection, RootCertStore, StreamOwned};
use rustls_pki_types::{CertificateDer, PrivateKeyDer, ServerName, pem::PemObject};
use serde_json::{Value, json};

/// A real, deterministic MCP tool: returns `{"sum": a + b}` for integer inputs.
/// Genuine arithmetic — the transport carries actual computed output, not a
/// canned fixture.
struct AdderTool;

impl Tool for AdderTool {
    fn def(&self) -> ToolDef {
        ToolDef {
            name: "adder".into(),
            description: "add two integers".into(),
            use_when: "you need a deterministic transport round-trip probe".into(),
            input_schema: json!({
                "type": "object",
                "properties": { "a": {"type": "integer"}, "b": {"type": "integer"} },
                "required": ["a", "b"],
            }),
        }
    }

    fn call(&self, params: Value) -> ToolResult<Value> {
        let a = params
            .get("a")
            .and_then(Value::as_i64)
            .ok_or_else(|| CalyxError {
                code: "CALYX_MCP_JSONRPC_INVALID",
                message: "adder requires integer `a`".into(),
                remediation: "pass integer arguments a and b",
            })?;
        let b = params
            .get("b")
            .and_then(Value::as_i64)
            .ok_or_else(|| CalyxError {
                code: "CALYX_MCP_JSONRPC_INVALID",
                message: "adder requires integer `b`".into(),
                remediation: "pass integer arguments a and b",
            })?;
        Ok(json!({ "sum": a + b }))
    }

    fn requires_authn(&self) -> bool {
        false
    }
}

#[derive(Clone)]
struct TestIdentity {
    mtls: MtlsConfig,
    cert_pem: String,
    key_pem: String,
}

type TlsClient = StreamOwned<ClientConnection, TcpStream>;

static NEXT_MTLS_ID: AtomicU64 = AtomicU64::new(0);

/// Boots a server with `AdderTool` registered on an OS-assigned loopback port.
fn boot_server() -> (
    SocketAddr,
    ShutdownHandle,
    std::thread::JoinHandle<()>,
    TestIdentity,
) {
    let identity = test_identity();
    let mut dispatcher = McpServer::new();
    dispatcher
        .register(Box::new(AdderTool))
        .expect("register adder tool");
    let server = CalyxMcpServer::bind(
        "127.0.0.1:0".parse().unwrap(),
        Arc::new(dispatcher),
        identity.mtls.clone(),
    )
    .unwrap();
    let addr = server.local_addr().unwrap();
    let handle = server.shutdown_handle().unwrap();
    let join = std::thread::spawn(move || {
        server.run().expect("server run");
    });
    (addr, handle, join, identity)
}

fn test_identity() -> TestIdentity {
    let CertifiedKey { cert, signing_key } =
        generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();
    let cert_pem = cert.pem();
    let key_pem = signing_key.serialize_pem();
    let nonce = NEXT_MTLS_ID.fetch_add(1, Ordering::SeqCst);
    let root = std::env::temp_dir().join(format!(
        "calyxd_mcp_integration_{}_{}",
        std::process::id(),
        nonce
    ));
    std::fs::create_dir_all(&root).unwrap();
    let cert_path = root.join("server-cert.pem");
    let key_path = root.join("server-key.pem");
    let ca_path = root.join("client-ca.pem");
    std::fs::write(&cert_path, &cert_pem).unwrap();
    std::fs::write(&key_path, &key_pem).unwrap();
    std::fs::write(&ca_path, &cert_pem).unwrap();
    TestIdentity {
        mtls: MtlsConfig {
            tls: TlsConfig {
                cert_pem_path: cert_path,
                key_pem_path: key_path,
                ca_pem_path: Some(ca_path),
            },
            require_client_cert: true,
        },
        cert_pem,
        key_pem,
    }
}

fn connect_tls(addr: SocketAddr, identity: &TestIdentity) -> TlsClient {
    let mut roots = RootCertStore::empty();
    let (added, ignored) = roots.add_parsable_certificates(certs_from_pem(&identity.cert_pem));
    assert_eq!(added, 1);
    assert_eq!(ignored, 0);
    let config = ClientConfig::builder()
        .with_root_certificates(roots)
        .with_client_auth_cert(
            certs_from_pem(&identity.cert_pem),
            key_from_pem(&identity.key_pem),
        )
        .unwrap();
    let server_name = ServerName::try_from("localhost").unwrap().to_owned();
    let conn = ClientConnection::new(Arc::new(config), server_name).unwrap();
    let mut client = StreamOwned::new(conn, TcpStream::connect(addr).unwrap());
    while client.conn.is_handshaking() {
        client.conn.complete_io(&mut client.sock).unwrap();
    }
    client
}

fn certs_from_pem(pem: &str) -> Vec<CertificateDer<'static>> {
    CertificateDer::pem_slice_iter(pem.as_bytes())
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
}

fn key_from_pem(pem: &str) -> PrivateKeyDer<'static> {
    PrivateKeyDer::from_pem_slice(pem.as_bytes()).unwrap()
}

fn send_frame(stream: &mut impl Write, payload: &[u8]) {
    let len = u32::try_from(payload.len()).unwrap().to_be_bytes();
    stream.write_all(&len).unwrap();
    stream.write_all(payload).unwrap();
    stream.flush().unwrap();
}

fn recv_frame(stream: &mut impl Read) -> Vec<u8> {
    let mut len = [0_u8; 4];
    stream.read_exact(&mut len).unwrap();
    let n = u32::from_be_bytes(len) as usize;
    let mut body = vec![0_u8; n];
    stream.read_exact(&mut body).unwrap();
    body
}

fn round_trip(stream: &mut TlsClient, request: &Value) -> Value {
    let bytes = serde_json::to_vec(request).unwrap();
    send_frame(stream, &bytes);
    let response = recv_frame(stream);
    serde_json::from_slice(&response).unwrap()
}

#[test]
fn search_style_tool_call_round_trips_with_hand_computed_result() {
    let (addr, handle, join, identity) = boot_server();
    let mut client = connect_tls(addr, &identity);

    // initialize: a real MCP handshake against the real dispatcher.
    let init = round_trip(
        &mut client,
        &json!({"jsonrpc":"2.0","id":1,"method":"initialize"}),
    );
    assert_eq!(init["result"]["serverInfo"]["name"], "calyx-mcp");

    // tools/list: the registered adder must be advertised.
    let listed = round_trip(
        &mut client,
        &json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}),
    );
    let tools = listed["result"]["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0]["name"], "adder");

    // tools/call adder{a:2,b:2}: hand-computed expected sum is 4.
    let called = round_trip(
        &mut client,
        &json!({
            "jsonrpc":"2.0","id":3,"method":"tools/call",
            "params":{"name":"adder","arguments":{"a":2,"b":2}}
        }),
    );
    let text = called["result"]["content"][0]["text"].as_str().unwrap();
    let payload: Value = serde_json::from_str(text).unwrap();
    assert_eq!(payload["sum"], 4, "2 + 2 must equal 4 over the wire");

    handle.shutdown();
    join.join().unwrap();
}

#[test]
fn unknown_tool_returns_method_not_found_and_connection_survives() {
    let (addr, handle, join, identity) = boot_server();
    let mut client = connect_tls(addr, &identity);

    // The card's `CALYX_MCP_UNKNOWN_TOOL` is realized as the existing calyx-mcp
    // contract: JSON-RPC -32601 method-not-found (the transport does not fork the
    // dispatcher's error taxonomy).
    let unknown = round_trip(
        &mut client,
        &json!({
            "jsonrpc":"2.0","id":1,"method":"tools/call",
            "params":{"name":"does-not-exist"}
        }),
    );
    assert_eq!(unknown["error"]["code"], -32601);

    // Connection must remain open: a second valid request still gets served.
    let after = round_trip(
        &mut client,
        &json!({
            "jsonrpc":"2.0","id":2,"method":"tools/call",
            "params":{"name":"adder","arguments":{"a":40,"b":2}}
        }),
    );
    let text = after["result"]["content"][0]["text"].as_str().unwrap();
    let payload: Value = serde_json::from_str(text).unwrap();
    assert_eq!(payload["sum"], 42);

    handle.shutdown();
    join.join().unwrap();
}

#[test]
fn malformed_json_frame_yields_error_response_and_keeps_connection() {
    let (addr, handle, join, identity) = boot_server();
    let mut client = connect_tls(addr, &identity);

    // A complete frame whose body is not valid JSON-RPC.
    send_frame(&mut client, b"this is not json");
    let response: Value = serde_json::from_slice(&recv_frame(&mut client)).unwrap();
    assert_eq!(
        response["error"]["data"]["calyx_code"],
        "CALYX_MCP_JSONRPC_INVALID"
    );
    assert_eq!(response["id"], Value::Null);

    // Connection stays open for the next, valid frame.
    let after = round_trip(
        &mut client,
        &json!({
            "jsonrpc":"2.0","id":7,"method":"tools/call",
            "params":{"name":"adder","arguments":{"a":1,"b":1}}
        }),
    );
    let text = after["result"]["content"][0]["text"].as_str().unwrap();
    let payload: Value = serde_json::from_str(text).unwrap();
    assert_eq!(payload["sum"], 2);

    handle.shutdown();
    join.join().unwrap();
}

#[test]
fn notification_without_id_receives_no_reply() {
    let (addr, handle, join, identity) = boot_server();
    let mut client = connect_tls(addr, &identity);

    // A JSON-RPC notification (no `id`) must produce no response frame. Send one,
    // then a real request; the only frame we read back must be the request's.
    send_frame(
        &mut client,
        &serde_json::to_vec(&json!({"jsonrpc":"2.0","method":"tools/list"})).unwrap(),
    );
    let reply = round_trip(
        &mut client,
        &json!({"jsonrpc":"2.0","id":99,"method":"tools/list"}),
    );
    assert_eq!(
        reply["id"], 99,
        "the only reply must be the id=99 request's"
    );

    handle.shutdown();
    join.join().unwrap();
}

#[test]
fn plaintext_jsonrpc_frame_is_rejected_before_dispatch() {
    let (addr, handle, join, _identity) = boot_server();
    let mut client = TcpStream::connect(addr).unwrap();
    client
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    send_frame(
        &mut client,
        b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/list\"}",
    );
    let mut prefix = [0_u8; 4];
    if client.read_exact(&mut prefix).is_ok() {
        let claimed_len = u32::from_be_bytes(prefix);
        assert!(
            claimed_len > 1024,
            "plaintext must not receive a small length-prefixed JSON-RPC response"
        );
    }

    handle.shutdown();
    join.join().unwrap();
}

#[test]
fn pre_handshake_disconnect_leaves_no_leaked_connection() {
    let (addr, handle, join, _identity) = boot_server();

    // Send two non-TLS bytes, then drop the socket before any MCP frame exists.
    {
        let mut client = TcpStream::connect(addr).unwrap();
        client.write_all(&[0x00, 0x01]).unwrap();
        client.flush().unwrap();
        // Give the server thread a beat to accept and increment the counter.
        wait_until(Duration::from_secs(2), || handle.active_connections() >= 1);
    }
    // After the client drops, the TLS handler errors and exits:
    // the live-connection count must return to 0 (no leak).
    assert!(
        wait_until(Duration::from_secs(3), || handle.active_connections() == 0),
        "connection count did not drain to 0 after disconnect"
    );

    handle.shutdown();
    join.join().unwrap();
}

#[test]
fn shutdown_stops_accepting_new_connections() {
    let (addr, handle, join, identity) = boot_server();
    // Prove it serves first.
    {
        let mut client = connect_tls(addr, &identity);
        let reply = round_trip(
            &mut client,
            &json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}),
        );
        assert!(reply["result"]["tools"].is_array());
    }
    handle.shutdown();
    join.join().unwrap();

    // After the accept loop returns the listener is dropped; the port is closed,
    // so a fresh connect either refuses or yields a socket that reads EOF.
    if let Ok(mut late) = TcpStream::connect(addr) {
        late.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        send_frame(
            &mut late,
            b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/list\"}",
        );
        let mut len = [0_u8; 4];
        assert!(
            late.read_exact(&mut len).is_err(),
            "a connection after shutdown must not be served"
        );
    }
}

/// Polls `cond` until it returns true or `budget` elapses; returns the last
/// observed value. Avoids a fixed sleep so the tests stay fast and deterministic.
fn wait_until(budget: Duration, mut cond: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + budget;
    loop {
        if cond() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}
