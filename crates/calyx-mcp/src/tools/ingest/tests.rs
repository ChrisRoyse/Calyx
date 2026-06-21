use std::ffi::OsString;
use std::fs;
use std::path::PathBuf;
use std::sync::MutexGuard;

use calyx_core::AuthN;
use serde_json::{Value, json};

use crate::jsonrpc::decode_jsonrpc_request;
use crate::protocol::JsonRpcError;
use crate::server::McpServer;
use crate::tools::test_support::ENV_LOCK;

struct TestEnv {
    home: PathBuf,
    old_home: Option<OsString>,
    _guard: MutexGuard<'static, ()>,
}

impl TestEnv {
    fn new(name: &str) -> Self {
        let guard = ENV_LOCK.lock().unwrap();
        let home =
            std::env::temp_dir().join(format!("calyx-mcp-ingest-{name}-{}", std::process::id()));
        if home.exists() {
            fs::remove_dir_all(&home).expect("remove stale test home");
        }
        fs::create_dir_all(&home).expect("create test home");
        let old_home = std::env::var_os("CALYX_HOME");
        unsafe {
            std::env::set_var("CALYX_HOME", &home);
        }
        Self {
            home,
            old_home,
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
        if self.home.starts_with(std::env::temp_dir()) {
            let _ = fs::remove_dir_all(&self.home);
        }
    }
}

fn server() -> McpServer {
    let mut server = McpServer::new();
    crate::tools::register_all(&mut server).unwrap();
    server
}

fn authn() -> AuthN {
    AuthN::InProcess {
        host_app_id: "calyx-mcp-test".into(),
    }
}

fn call_ok(server: &McpServer, id: u64, name: &str, arguments: Value) -> Value {
    let request = decode_jsonrpc_request(
        json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "tools/call",
            "params": { "name": name, "arguments": arguments }
        })
        .to_string()
        .as_bytes(),
    )
    .unwrap();
    let authn = authn();
    let response = server.dispatch_with_authn(request, Some(&authn));
    assert!(response.error.is_none(), "{:?}", response.error);
    let result = response.result.unwrap();
    let text = result["content"][0]["text"].as_str().unwrap();
    serde_json::from_str(text).unwrap()
}

fn call_err(server: &McpServer, id: u64, name: &str, arguments: Value) -> JsonRpcError {
    let request = decode_jsonrpc_request(
        json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "tools/call",
            "params": { "name": name, "arguments": arguments }
        })
        .to_string()
        .as_bytes(),
    )
    .unwrap();
    let authn = authn();
    server
        .dispatch_with_authn(request, Some(&authn))
        .error
        .unwrap()
}

fn vault_with_algorithmic(server: &McpServer, name: &str) {
    call_ok(server, 1, "calyx.create_vault", json!({"name": name}));
    call_ok(
        server,
        2,
        "calyx.add_lens",
        json!({"vault": name, "name": "byte_axis", "runtime": "algorithmic"}),
    );
}

#[test]
fn ingest_twice_is_idempotent_and_returns_retry_ledger_seq() {
    let _env = TestEnv::new("idempotent");
    let server = server();
    vault_with_algorithmic(&server, "v");

    let first = call_ok(
        &server,
        3,
        "calyx.ingest",
        json!({"vault": "v", "input": "hello"}),
    );
    let second = call_ok(
        &server,
        4,
        "calyx.ingest",
        json!({"vault": "v", "input": "hello"}),
    );

    assert_eq!(first["cx_id"], second["cx_id"]);
    assert_eq!(first["new"], true);
    assert_eq!(second["new"], false);
    assert!(second["ledger_seq"].as_u64().unwrap() > first["ledger_seq"].as_u64().unwrap());
}

#[test]
fn anchor_existing_constellation_returns_structured_ledger_seq() {
    let _env = TestEnv::new("anchor");
    let server = server();
    vault_with_algorithmic(&server, "v");
    let ingested = call_ok(
        &server,
        5,
        "calyx.ingest",
        json!({"vault": "v", "input": "anchored"}),
    );

    let anchored = call_ok(
        &server,
        6,
        "calyx.anchor",
        json!({
            "vault": "v",
            "cx_id": ingested["cx_id"],
            "kind": "test_pass",
            "value": true,
            "confidence": 1.0
        }),
    );

    assert_eq!(anchored["status"], "anchored");
    assert_eq!(anchored["cx_id"], ingested["cx_id"]);
    assert!(anchored["ledger_seq"].as_u64().unwrap() > ingested["ledger_seq"].as_u64().unwrap());
}

#[test]
fn measure_returns_slot_array_with_absent_vectors_not_zero_fill() {
    let _env = TestEnv::new("measure");
    let server = server();
    vault_with_algorithmic(&server, "v");

    let measured = call_ok(
        &server,
        7,
        "calyx.measure",
        json!({"vault": "v", "input": "measure me"}),
    );

    let slots = measured["slots"].as_array().unwrap();
    assert!(
        slots
            .iter()
            .any(|slot| slot["vector"].get("dense").is_some())
    );
    assert!(slots.iter().any(|slot| {
        slot["vector"]["absent"]["reason"] == "lens_unavailable"
            && slot["vector"].get("dense").is_none()
    }));
}

#[test]
fn ingest_batch_returns_three_distinct_results() {
    let _env = TestEnv::new("batch");
    let server = server();
    vault_with_algorithmic(&server, "v");

    let result = call_ok(
        &server,
        8,
        "calyx.ingest",
        json!({"vault": "v", "batch": ["one", "two", "three"]}),
    );

    let rows = result["results"].as_array().unwrap();
    assert_eq!(rows.len(), 3);
    let ids = rows
        .iter()
        .map(|row| row["cx_id"].as_str().unwrap())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(ids.len(), 3);
    assert!(rows.iter().all(|row| row["new"] == true));
}

#[test]
fn ingest_with_input_and_batch_is_invalid_params() {
    let _env = TestEnv::new("both");
    let server = server();
    vault_with_algorithmic(&server, "v");

    let error = call_err(
        &server,
        9,
        "calyx.ingest",
        json!({"vault": "v", "input": "x", "batch": ["y"]}),
    );

    assert_eq!(error.code, -32602);
    assert!(error.message.contains("mutually exclusive"));
}

#[test]
fn ingest_media_rejects_unsupported_extension_before_write() {
    let env = TestEnv::new("media-unsupported");
    let server = server();
    call_ok(
        &server,
        31,
        "calyx.create_vault",
        json!({"name": "media", "panel_template": "media-default"}),
    );
    let media_path = env.home.join("clip.txt");
    fs::write(&media_path, b"not a video").unwrap();

    let error = call_err(
        &server,
        32,
        "calyx.ingest_media",
        json!({"vault": "media", "file": media_path, "modality": "video"}),
    );

    assert_eq!(error.code, -32000);
    assert_eq!(
        error.data.unwrap()["calyx_code"],
        "CALYX_MEDIA_UNSUPPORTED_EXTENSION"
    );
    let retained_inputs_exist = fs::read_dir(env.home.join("vaults"))
        .unwrap()
        .any(|entry| entry.unwrap().path().join("inputs").exists());
    assert!(!retained_inputs_exist);
}

#[test]
fn label_anchor_requires_label_field() {
    let _env = TestEnv::new("label");
    let server = server();
    vault_with_algorithmic(&server, "v");
    let ingested = call_ok(
        &server,
        10,
        "calyx.ingest",
        json!({"vault": "v", "input": "labeled"}),
    );

    let error = call_err(
        &server,
        11,
        "calyx.anchor",
        json!({"vault": "v", "cx_id": ingested["cx_id"], "kind": "label", "value": true}),
    );

    assert_eq!(error.code, -32602);
    assert!(error.message.contains("requires label"));
}

#[test]
fn anchor_unknown_cx_is_vault_access_denied() {
    let _env = TestEnv::new("unknown-cx");
    let server = server();
    vault_with_algorithmic(&server, "v");

    let error = call_err(
        &server,
        12,
        "calyx.anchor",
        json!({
            "vault": "v",
            "cx_id": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "kind": "test_pass",
            "value": true
        }),
    );

    assert_eq!(error.code, -32000);
    assert_eq!(
        error.data.unwrap()["calyx_code"],
        "CALYX_VAULT_ACCESS_DENIED"
    );
}

#[test]
fn ingest_with_all_applicable_lenses_unavailable_fails_closed() {
    let _env = TestEnv::new("unavailable");
    let server = server();
    call_ok(&server, 13, "calyx.create_vault", json!({"name": "v"}));

    let error = call_err(
        &server,
        14,
        "calyx.ingest",
        json!({"vault": "v", "input": "no runtime"}),
    );

    assert_eq!(error.code, -32000);
    let data = error.data.unwrap();
    assert_eq!(data["calyx_code"], "CALYX_LENS_UNREACHABLE");
    assert_eq!(data["remediation"], "restore lens service");
}
