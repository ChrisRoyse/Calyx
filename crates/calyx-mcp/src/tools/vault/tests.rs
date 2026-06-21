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
            std::env::temp_dir().join(format!("calyx-mcp-vault-{name}-{}", std::process::id()));
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
    super::register(&mut server).unwrap();
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

#[test]
fn create_vault_code_default_returns_vault_id_and_template() {
    let _env = TestEnv::new("create");
    let server = server();

    let result = call_ok(
        &server,
        1,
        "calyx.create_vault",
        json!({"name": "t", "panel_template": "code-default"}),
    );

    assert!(!result["vault_id"].as_str().unwrap().is_empty());
    assert_eq!(result["panel_template"], "code-default");
}

#[test]
fn add_lens_missing_vault_is_invalid_params() {
    let _env = TestEnv::new("missing-vault");
    let server = server();

    let error = call_err(
        &server,
        2,
        "calyx.add_lens",
        json!({"name": "byte_axis", "runtime": "algorithmic"}),
    );

    assert_eq!(error.code, -32602);
    assert!(error.message.contains("missing field `vault`"));
}

#[test]
fn add_lens_algorithmic_returns_active_lens_and_list_panel_sees_it() {
    let _env = TestEnv::new("add-list");
    let server = server();
    call_ok(&server, 3, "calyx.create_vault", json!({"name": "panel"}));

    let added = call_ok(
        &server,
        4,
        "calyx.add_lens",
        json!({"vault": "panel", "name": "byte_axis", "runtime": "algorithmic"}),
    );
    let lens_id = added["lens_id"].as_str().unwrap();
    assert_eq!(lens_id.len(), 32);
    assert!(lens_id.bytes().all(|byte| byte.is_ascii_hexdigit()));
    assert!(added["slot_id"].as_u64().unwrap() <= u16::MAX as u64);
    assert_eq!(added["state"], "active");

    let listed = call_ok(&server, 5, "calyx.list_panel", json!({"vault": "panel"}));
    let slots = listed["slots"].as_array().unwrap();
    assert!(slots.iter().any(|slot| {
        slot["name"] == "byte_axis"
            && slot["state"] == "active"
            && slot["lens_id"] == added["lens_id"]
    }));
}

#[test]
fn add_lens_algorithmic_video_preserves_modality_in_panel() {
    let _env = TestEnv::new("add-video");
    let server = server();
    call_ok(
        &server,
        31,
        "calyx.create_vault",
        json!({"name": "panel", "panel_template": "media-default"}),
    );

    let added = call_ok(
        &server,
        32,
        "calyx.add_lens",
        json!({
            "vault": "panel",
            "name": "video_bytes",
            "runtime": "algorithmic",
            "modality": "video",
            "shape": "Dense(16)"
        }),
    );
    let listed = call_ok(&server, 33, "calyx.list_panel", json!({"vault": "panel"}));
    let slots = listed["slots"].as_array().unwrap();
    assert!(slots.iter().any(|slot| {
        slot["name"] == "video_bytes"
            && slot["modality"] == "video"
            && slot["lens_id"] == added["lens_id"]
    }));
}

#[test]
fn unknown_panel_template_is_invalid_params() {
    let _env = TestEnv::new("unknown-panel");
    let server = server();

    let error = call_err(
        &server,
        6,
        "calyx.create_vault",
        json!({"name": "bad", "panel_template": "unknown-panel"}),
    );

    assert_eq!(error.code, -32602);
    assert!(error.message.contains("unknown panel_template"));
}

#[test]
fn retire_nonexistent_slot_maps_to_vault_access_denied() {
    let _env = TestEnv::new("missing-slot");
    let server = server();
    call_ok(&server, 7, "calyx.create_vault", json!({"name": "panel"}));

    let error = call_err(
        &server,
        8,
        "calyx.retire_lens",
        json!({"vault": "panel", "slot": 65000}),
    );

    assert_eq!(error.code, -32000);
    let data = error.data.unwrap();
    assert_eq!(data["calyx_code"], "CALYX_VAULT_ACCESS_DENIED");
}

#[test]
fn add_lens_tei_http_without_endpoint_fails_closed() {
    let _env = TestEnv::new("tei-no-endpoint");
    let server = server();
    call_ok(&server, 9, "calyx.create_vault", json!({"name": "panel"}));

    let error = call_err(
        &server,
        10,
        "calyx.add_lens",
        json!({"vault": "panel", "name": "tei", "runtime": "tei-http"}),
    );

    assert_eq!(error.code, -32000);
    let data = error.data.unwrap();
    assert_eq!(data["calyx_code"], "CALYX_LENS_UNREACHABLE");
    assert_eq!(data["remediation"], "restore lens service");
}
