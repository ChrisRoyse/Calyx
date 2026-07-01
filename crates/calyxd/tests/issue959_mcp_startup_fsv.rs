//! Issue #959: production MCP tools over the calyxd loopback mTLS socket.
//!
//! Source of truth is the durable `CALYX_HOME/vaults` tree. The test drives
//! real JSON-RPC frames over a real TLS socket, then separately reads the vault
//! index and vault files from disk.

use std::ffi::OsString;
use std::fs;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use calyx_core::{MtlsConfig, TlsConfig};
use calyx_mcp::McpServer;
use calyxd::mcp_server::{CalyxMcpServer, ShutdownHandle};
use rcgen::{CertifiedKey, generate_simple_self_signed};
use rustls::{ClientConfig, ClientConnection, RootCertStore, StreamOwned};
use rustls_pki_types::{CertificateDer, PrivateKeyDer, ServerName, pem::PemObject};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

type TlsClient = StreamOwned<ClientConnection, TcpStream>;

static ENV_LOCK: Mutex<()> = Mutex::new(());
static NEXT_ID: AtomicU64 = AtomicU64::new(0);

#[test]
fn issue959_production_mcp_socket_tools_and_vault_readback_fsv() {
    let env = TestEnv::new("issue959-production-mcp");
    let before = snapshot(&env.home);
    let (addr, shutdown, join, identity) = boot_production_server();
    let mut client = connect_tls(addr, &identity);

    let initialized = round_trip(
        &mut client,
        &json!({"jsonrpc":"2.0","id":1,"method":"initialize"}),
    );
    assert_eq!(initialized["result"]["serverInfo"]["name"], "calyx-mcp");

    let listed = round_trip(
        &mut client,
        &json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}),
    );
    let tools = listed["result"]["tools"].as_array().unwrap();
    assert!(
        tools.len() >= 31,
        "expected production tools, got {tools:?}"
    );
    for required in [
        "calyx.create_vault",
        "calyx.ingest",
        "calyx.search",
        "calyx.provenance",
    ] {
        assert!(
            tools.iter().any(|tool| tool["name"] == required),
            "missing production tool {required}"
        );
    }

    let created = call_tool(
        &mut client,
        3,
        "calyx.create_vault",
        json!({"name": "issue959", "panel_template": "text-default"}),
    );
    let vault_id = created["vault_id"].as_str().unwrap().to_string();
    call_tool(
        &mut client,
        4,
        "calyx.add_lens",
        json!({"vault": "issue959", "name": "byte_axis", "runtime": "algorithmic"}),
    );
    let ingested = call_tool(
        &mut client,
        5,
        "calyx.ingest",
        json!({"vault": "issue959", "input": "issue 959 production MCP socket proof"}),
    );
    let cx_id = ingested["cx_id"].as_str().unwrap().to_string();
    let searched = call_tool(
        &mut client,
        6,
        "calyx.search",
        json!({"vault": "issue959", "query": "production MCP socket proof", "k": 1}),
    );
    assert!(!searched["hits"].as_array().unwrap().is_empty());
    let provenance = call_tool(
        &mut client,
        7,
        "calyx.provenance",
        json!({"vault": "issue959", "cx_id": cx_id}),
    );
    assert_eq!(provenance["cx_id"], ingested["cx_id"]);

    client.conn.send_close_notify();
    client.flush().unwrap();
    drop(client);
    shutdown.shutdown();
    join.join().unwrap().unwrap();

    let index_path = env.home.join("vaults").join("index.json");
    let index: Value = serde_json::from_slice(&fs::read(&index_path).unwrap()).unwrap();
    assert_eq!(index["vaults"][0]["name"], "issue959");
    assert_eq!(index["vaults"][0]["vault_id"], vault_id);
    let vault_dir = env.home.join("vaults").join(&vault_id);
    assert!(
        vault_dir.exists(),
        "vault dir missing: {}",
        vault_dir.display()
    );
    let after = snapshot(&env.home);
    assert!(after["files"].as_array().unwrap().len() > before["files"].as_array().unwrap().len());

    let readback = json!({
        "issue": 959,
        "trigger": "production calyxd MCP socket over mTLS",
        "source_of_truth": {
            "calyx_home": env.home,
            "index_path": index_path,
            "vault_dir": vault_dir
        },
        "before": before,
        "after": after,
        "tools_count": tools.len(),
        "created": created,
        "ingested": ingested,
        "search_hit_count": searched["hits"].as_array().unwrap().len(),
        "provenance": provenance
    });
    let out = write_readback("issue959-production-mcp-readback.json", &readback);
    println!("ISSUE959_READBACK={}", out.display());
    println!("ISSUE959_READBACK_SHA256={}", sha256_file(&out));
}

fn boot_production_server() -> (
    SocketAddr,
    ShutdownHandle,
    std::thread::JoinHandle<Result<(), calyxd::error::DaemonError>>,
    TestIdentity,
) {
    let identity = TestIdentity::new();
    let mut dispatcher = McpServer::new();
    calyx_mcp::tools::register_all(&mut dispatcher).unwrap();
    let server = CalyxMcpServer::bind(
        "127.0.0.1:0".parse().unwrap(),
        Arc::new(dispatcher),
        identity.mtls.clone(),
    )
    .unwrap();
    let addr = server.local_addr().unwrap();
    let shutdown = server.shutdown_handle().unwrap();
    let join = std::thread::spawn(move || server.run());
    (addr, shutdown, join, identity)
}

struct TestIdentity {
    mtls: MtlsConfig,
    cert_pem: String,
    key_pem: String,
}

impl TestIdentity {
    fn new() -> Self {
        let CertifiedKey { cert, signing_key } =
            generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();
        let cert_pem = cert.pem();
        let key_pem = signing_key.serialize_pem();
        let id = NEXT_ID.fetch_add(1, Ordering::SeqCst);
        let root =
            std::env::temp_dir().join(format!("calyxd-issue959-mtls-{}-{id}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let cert_path = root.join("server-cert.pem");
        let key_path = root.join("server-key.pem");
        let ca_path = root.join("client-ca.pem");
        fs::write(&cert_path, &cert_pem).unwrap();
        fs::write(&key_path, &key_pem).unwrap();
        fs::write(&ca_path, &cert_pem).unwrap();
        Self {
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
}

struct TestEnv {
    home: PathBuf,
    old_home: Option<OsString>,
    preserve_home: bool,
    _guard: MutexGuard<'static, ()>,
}

impl TestEnv {
    fn new(name: &str) -> Self {
        let guard = ENV_LOCK.lock().unwrap();
        let id = NEXT_ID.fetch_add(1, Ordering::SeqCst);
        let fsv_root = calyx_fsv::fsv_root("CALYX_FSV_ROOT");
        let preserve_home = fsv_root.is_some();
        let home = fsv_root
            .map(|root| root.join("calyx-home"))
            .unwrap_or_else(|| {
                std::env::temp_dir().join(format!("calyxd-{name}-{}-{id}", std::process::id()))
            });
        let _ = fs::remove_dir_all(&home);
        fs::create_dir_all(&home).unwrap();
        let old_home = std::env::var_os("CALYX_HOME");
        unsafe {
            std::env::set_var("CALYX_HOME", &home);
        }
        Self {
            home,
            old_home,
            preserve_home,
            _guard: guard,
        }
    }
}

impl Drop for TestEnv {
    fn drop(&mut self) {
        match &self.old_home {
            Some(value) => unsafe {
                std::env::set_var("CALYX_HOME", value);
            },
            None => unsafe {
                std::env::remove_var("CALYX_HOME");
            },
        }
        if !self.preserve_home && self.home.starts_with(std::env::temp_dir()) {
            let _ = fs::remove_dir_all(&self.home);
        }
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
    client
        .sock
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
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

fn round_trip(stream: &mut TlsClient, request: &Value) -> Value {
    let bytes = serde_json::to_vec(request).unwrap();
    send_frame(stream, &bytes);
    serde_json::from_slice(&recv_frame(stream)).unwrap()
}

fn call_tool(stream: &mut TlsClient, id: u64, name: &str, arguments: Value) -> Value {
    let response = round_trip(
        stream,
        &json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "tools/call",
            "params": {"name": name, "arguments": arguments}
        }),
    );
    assert!(response.get("error").is_none(), "{response}");
    let text = response["result"]["content"][0]["text"].as_str().unwrap();
    serde_json::from_str(text).unwrap()
}

fn send_frame(stream: &mut impl Write, payload: &[u8]) {
    stream
        .write_all(&u32::try_from(payload.len()).unwrap().to_be_bytes())
        .unwrap();
    stream.write_all(payload).unwrap();
    stream.flush().unwrap();
}

fn recv_frame(stream: &mut impl Read) -> Vec<u8> {
    let mut len = [0_u8; 4];
    stream.read_exact(&mut len).unwrap();
    let mut body = vec![0_u8; u32::from_be_bytes(len) as usize];
    stream.read_exact(&mut body).unwrap();
    body
}

fn snapshot(root: &Path) -> Value {
    let mut files = Vec::new();
    if root.exists() {
        collect_files(root, root, &mut files);
    }
    files.sort_by(|left, right| {
        left["relative"]
            .as_str()
            .unwrap()
            .cmp(right["relative"].as_str().unwrap())
    });
    json!({"exists": root.exists(), "files": files})
}

fn collect_files(root: &Path, path: &Path, out: &mut Vec<Value>) {
    for entry in fs::read_dir(path).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            collect_files(root, &path, out);
        } else {
            let bytes = fs::read(&path).unwrap();
            out.push(json!({
                "relative": path.strip_prefix(root).unwrap().display().to_string(),
                "len": bytes.len(),
                "sha256": sha256_bytes(&bytes)
            }));
        }
    }
}

fn write_readback(name: &str, value: &Value) -> PathBuf {
    let root = calyx_fsv::fsv_root_or_else("CALYX_FSV_ROOT", || {
        std::env::temp_dir().join("calyxd-issue959-fsv")
    });
    fs::create_dir_all(&root).unwrap();
    let out = root.join(name);
    fs::write(&out, serde_json::to_vec_pretty(value).unwrap()).unwrap();
    out
}

fn sha256_file(path: &Path) -> String {
    sha256_bytes(&fs::read(path).unwrap())
}

fn sha256_bytes(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}
