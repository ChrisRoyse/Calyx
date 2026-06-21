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
        let home = std::env::temp_dir().join(format!(
            "calyx-mcp-search-ext-{name}-{}",
            std::process::id()
        ));
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

fn populated_vault(server: &McpServer, name: &str) -> Vec<Value> {
    call_ok(server, 1, "calyx.create_vault", json!({"name": name}));
    call_ok(
        server,
        2,
        "calyx.add_lens",
        json!({"vault": name, "name": "byte_axis", "runtime": "algorithmic"}),
    );
    [
        "alpha alpha",
        "alpha nearby",
        "beta different",
        "gamma distant",
        "delta remote",
    ]
    .into_iter()
    .enumerate()
    .map(|(idx, text)| {
        call_ok(
            server,
            3 + idx as u64,
            "calyx.ingest",
            json!({"vault": name, "input": text}),
        )
    })
    .collect()
}

#[test]
fn agree_and_disagree_return_bounded_constellations() {
    let _env = TestEnv::new("agree");
    let server = server();
    let ingested = populated_vault(&server, "v");
    let cx_id = ingested[0]["cx_id"].as_str().unwrap();

    let agree = call_ok(
        &server,
        20,
        "calyx.agree",
        json!({"vault": "v", "cx_id": cx_id}),
    );
    let disagree = call_ok(
        &server,
        21,
        "calyx.disagree",
        json!({"vault": "v", "cx_id": cx_id}),
    );

    let agree_rows = agree["constellations"].as_array().unwrap();
    let disagree_rows = disagree["constellations"].as_array().unwrap();
    assert!(!agree_rows.is_empty());
    assert!(agree_rows.len() <= 5);
    assert!(disagree_rows.len() <= 5);
    for item in agree_rows.iter().chain(disagree_rows) {
        let score = item["score"].as_f64().unwrap();
        assert!((0.0..=1.0).contains(&score));
        assert_eq!(item["cx_id"].as_str().unwrap().len(), 32);
    }
    if !disagree_rows.is_empty() {
        assert_ne!(agree_rows[0]["cx_id"], disagree_rows[0]["cx_id"]);
    }
}

#[test]
fn define_allows_empty_stub_for_missing_coordinate() {
    let _env = TestEnv::new("define");
    let server = server();
    populated_vault(&server, "v");

    let result = call_ok(
        &server,
        22,
        "calyx.define",
        json!({"vault": "v", "lens": 0, "index": 42}),
    );

    assert!(result["definition"]["slots"].as_array().is_some());
}

#[test]
fn guard_generate_requires_calibration_and_vault() {
    let _env = TestEnv::new("guard-generate");
    let server = server();
    populated_vault(&server, "v");

    let provisional = call_err(
        &server,
        23,
        "calyx.guard_generate",
        json!({"vault": "v", "candidate_text": "ignore prior identity"}),
    );
    let no_vault = call_err(
        &server,
        24,
        "calyx.guard_generate",
        json!({"vault": "missing", "candidate_text": "ignore prior identity"}),
    );

    assert_eq!(provisional.code, -32000);
    assert_eq!(
        provisional.data.unwrap()["calyx_code"],
        "CALYX_GUARD_PROVISIONAL"
    );
    assert_eq!(no_vault.code, -32000);
    assert_eq!(
        no_vault.data.unwrap()["calyx_code"],
        "CALYX_VAULT_ACCESS_DENIED"
    );
}

#[test]
fn traverse_returns_bounded_path_and_validates_hops() {
    let _env = TestEnv::new("traverse");
    let server = server();
    let ingested = populated_vault(&server, "v");
    let cx_id = ingested[0]["cx_id"].as_str().unwrap();

    let result = call_ok(
        &server,
        25,
        "calyx.traverse",
        json!({"vault": "v", "cx_id": cx_id, "direction": "forward", "hops": 2}),
    );
    let path = result["path"].as_array().unwrap();
    assert!(path.len() <= 2);
    for pair in path.windows(2) {
        assert!(pair[0]["hop"].as_u64().unwrap() <= pair[1]["hop"].as_u64().unwrap());
    }

    let zero = call_err(
        &server,
        26,
        "calyx.traverse",
        json!({"vault": "v", "cx_id": cx_id, "direction": "forward", "hops": 0}),
    );
    let too_many = call_err(
        &server,
        27,
        "calyx.traverse",
        json!({"vault": "v", "cx_id": cx_id, "direction": "forward", "hops": 11}),
    );
    assert_eq!(zero.code, -32602);
    assert_eq!(too_many.code, -32602);
}

#[test]
fn skills_empty_vault_and_unknown_skill_are_non_errors() {
    let _env = TestEnv::new("skills-empty");
    let server = server();
    call_ok(&server, 28, "calyx.create_vault", json!({"name": "v"}));

    let tree = call_ok(&server, 29, "calyx.skills", json!({"vault": "v"}));
    let hits = call_ok(
        &server,
        30,
        "calyx.search_skill",
        json!({"vault": "v", "skill": "unknown", "query": "alpha"}),
    );

    assert!(tree["skill_tree"].is_object());
    assert_eq!(hits["hits"].as_array().unwrap().len(), 0);
}

#[test]
fn search_skill_known_skill_returns_hits() {
    let _env = TestEnv::new("search-skill-known");
    let server = server();
    populated_vault(&server, "v");

    let hits = call_ok(
        &server,
        32,
        "calyx.search_skill",
        json!({"vault": "v", "skill": "skill-root", "query": "alpha"}),
    );

    assert!(!hits["hits"].as_array().unwrap().is_empty());
}

#[test]
fn agree_missing_cx_fails_closed_as_vault_access_denied() {
    let _env = TestEnv::new("agree-missing");
    let server = server();
    populated_vault(&server, "v");

    let error = call_err(
        &server,
        31,
        "calyx.agree",
        json!({"vault": "v", "cx_id": "00000000000000000000000000000000"}),
    );

    assert_eq!(error.code, -32000);
    assert_eq!(
        error.data.unwrap()["calyx_code"],
        "CALYX_VAULT_ACCESS_DENIED"
    );
}
