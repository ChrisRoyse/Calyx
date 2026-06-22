use super::*;
use std::io::Cursor;
use std::sync::atomic::{AtomicU64, Ordering};

use calyx_core::TlsConfig;
use rcgen::{CertifiedKey, generate_simple_self_signed};

static NEXT_MTLS_ID: AtomicU64 = AtomicU64::new(0);

fn dispatcher() -> Arc<McpServer> {
    Arc::new(McpServer::new())
}

fn mtls_config() -> MtlsConfig {
    let CertifiedKey { cert, signing_key } =
        generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();
    let nonce = NEXT_MTLS_ID.fetch_add(1, Ordering::SeqCst);
    let root =
        std::env::temp_dir().join(format!("calyxd_mcp_mtls_{}_{}", std::process::id(), nonce));
    std::fs::create_dir_all(&root).unwrap();
    let cert_path = root.join("server-cert.pem");
    let key_path = root.join("server-key.pem");
    let ca_path = root.join("client-ca.pem");
    std::fs::write(&cert_path, cert.pem()).unwrap();
    std::fs::write(&key_path, signing_key.serialize_pem()).unwrap();
    std::fs::write(&ca_path, cert.pem()).unwrap();
    MtlsConfig {
        tls: TlsConfig {
            cert_pem_path: cert_path,
            key_pem_path: key_path,
            ca_pem_path: Some(ca_path),
        },
        require_client_cert: true,
    }
}

#[test]
fn bind_refuses_non_loopback_address() {
    let Err(error) =
        CalyxMcpServer::bind("0.0.0.0:7700".parse().unwrap(), dispatcher(), mtls_config())
    else {
        panic!("non-loopback bind must fail");
    };
    assert_eq!(error.code(), "CALYX_DAEMON_BIND_FAILED");
    assert!(error.to_string().contains("0.0.0.0:7700"));
}

#[test]
fn bind_accepts_ipv4_loopback() {
    let server =
        CalyxMcpServer::bind("127.0.0.1:0".parse().unwrap(), dispatcher(), mtls_config()).unwrap();
    assert!(server.local_addr().unwrap().ip().is_loopback());
}

#[test]
fn bind_accepts_ipv6_loopback() {
    let server =
        CalyxMcpServer::bind("[::1]:0".parse().unwrap(), dispatcher(), mtls_config()).unwrap();
    assert!(server.local_addr().unwrap().ip().is_loopback());
}

#[test]
fn from_config_requires_mtls_block() {
    let cfg = CalyxConfig::from_toml_str(
        "bind_addr = \"127.0.0.1:0\"\nvault_path = \"/v\"\nvram_budget_mib = 8192\nlog_dir = \"/l\"\n",
    )
    .unwrap();
    let Err(error) = CalyxMcpServer::from_config(&cfg, dispatcher()) else {
        panic!("from_config must require mcp_mtls");
    };
    assert_eq!(error.code(), "CALYX_TLS_CONFIG_INVALID");
    assert!(error.to_string().contains("mcp_mtls"));
}

#[test]
fn from_config_binds_when_mtls_present() {
    let cfg = CalyxConfig {
        bind_addr: "127.0.0.1:0".parse().unwrap(),
        vault_path: "/v".into(),
        vram_budget_mib: 8192,
        log_dir: "/l".into(),
        health_log_path: "/h".into(),
        tei_endpoints: Vec::new(),
        healthcheck_timeout_secs: 30,
        mcp_mtls: Some(mtls_config()),
        learner_origin: None,
    };
    let server = CalyxMcpServer::from_config(&cfg, dispatcher()).unwrap();
    assert!(server.local_addr().unwrap().ip().is_loopback());
}

#[test]
fn frame_round_trips_through_codec() {
    let payload = br#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#;
    let mut buf = Vec::new();
    write_frame(&mut buf, payload).unwrap();
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
    let mut cursor = Cursor::new(vec![0x00, 0x01]);
    let error = read_frame(&mut cursor).unwrap_err();
    assert!(error.contains(CALYX_DAEMON_FRAME_INVALID));
    assert!(error.contains("truncated"));
}

#[test]
fn read_frame_rejects_truncated_body() {
    let mut bytes = 8_u32.to_be_bytes().to_vec();
    bytes.extend_from_slice(b"abc");
    let mut cursor = Cursor::new(bytes);
    let error = read_frame(&mut cursor).unwrap_err();
    assert!(error.contains("frame body"));
}
