use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::MutexGuard;

use calyx_core::AuthN;
use serde_json::{Value, json};

use calyx_aster::cf::{CfRouter, ColumnFamily, ledger_key};

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
            std::env::temp_dir().join(format!("calyx-mcp-search-{name}-{}", std::process::id()));
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

    fn vault_path(&self, vault_id: &str) -> PathBuf {
        self.home.join("vaults").join(vault_id)
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

fn vault_with_algorithmic_data(server: &McpServer, name: &str) -> Vec<Value> {
    call_ok(server, 1, "calyx.create_vault", json!({"name": name}));
    call_ok(
        server,
        2,
        "calyx.add_lens",
        json!({"vault": name, "name": "byte_axis", "runtime": "algorithmic"}),
    );
    ["alpha", "beta"]
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

fn tamper_ledger_row(vault: &Path, seq: u64) {
    let mut router = CfRouter::open(vault, 0).expect("open CF router");
    let key = ledger_key(seq);
    let mut bytes = router
        .get(ColumnFamily::Ledger, &key)
        .expect("read ledger row")
        .expect("ledger row exists");
    let last = bytes.len().checked_sub(1).expect("non-empty ledger row");
    bytes[last] ^= 0x55;
    router
        .put(ColumnFamily::Ledger, &key, &bytes)
        .expect("write tampered ledger row");
    router
        .flush_cf(ColumnFamily::Ledger)
        .expect("flush tampered ledger row");
}

#[test]
fn minimal_search_returns_provenanced_hits() {
    let _env = TestEnv::new("minimal");
    let server = server();
    vault_with_algorithmic_data(&server, "v");

    let result = call_ok(
        &server,
        5,
        "calyx.search",
        json!({"vault": "v", "query": "alpha"}),
    );

    let hits = result["hits"].as_array().unwrap();
    assert!(!hits.is_empty());
    assert!(hits.iter().all(|hit| hit["provenance"].is_object()));
    assert!(hits.iter().all(|hit| hit["per_lens"].is_null()));
}

#[test]
fn search_fails_closed_when_ledger_chain_is_tampered() {
    let env = TestEnv::new("ledger-tamper");
    let server = server();
    let created = call_ok(&server, 1, "calyx.create_vault", json!({"name": "v"}));
    call_ok(
        &server,
        2,
        "calyx.add_lens",
        json!({"vault": "v", "name": "byte_axis", "runtime": "algorithmic"}),
    );
    call_ok(
        &server,
        3,
        "calyx.ingest",
        json!({"vault": "v", "input": "alpha"}),
    );
    let vault_id = created["vault_id"].as_str().unwrap();
    tamper_ledger_row(&env.vault_path(vault_id), 0);

    let error = call_err(
        &server,
        4,
        "calyx.search",
        json!({"vault": "v", "query": "alpha"}),
    );

    assert_eq!(error.code, -32000);
    assert_eq!(
        error.data.unwrap()["calyx_code"],
        "CALYX_LEDGER_CHAIN_BROKEN"
    );
}

#[test]
fn search_explain_includes_per_lens_breakdown() {
    let _env = TestEnv::new("explain");
    let server = server();
    vault_with_algorithmic_data(&server, "v");

    let result = call_ok(
        &server,
        6,
        "calyx.search",
        json!({"vault": "v", "query": "alpha", "explain": true}),
    );

    let first = &result["hits"].as_array().unwrap()[0];
    let per_lens = first["per_lens"].as_array().unwrap();
    assert!(!per_lens.is_empty());
    for field in ["slot", "rank", "raw", "weight", "contribution"] {
        assert!(per_lens[0].get(field).is_some(), "missing {field}");
    }
}

#[test]
fn kernel_answer_ungrounded_fails_closed() {
    let _env = TestEnv::new("kernel-ungrounded");
    let server = server();
    vault_with_algorithmic_data(&server, "v");

    let error = call_err(
        &server,
        7,
        "calyx.kernel_answer",
        json!({"vault": "v", "query": "alpha"}),
    );

    assert_eq!(error.code, -32000);
    let data = error.data.unwrap();
    assert_eq!(data["calyx_code"], "CALYX_KERNEL_UNGROUNDED");
    assert_eq!(data["remediation"], "add anchors (grounding_gaps)");
}

#[test]
fn neighbors_returns_bounded_scores_for_known_cx() {
    let _env = TestEnv::new("neighbors");
    let server = server();
    let ingested = vault_with_algorithmic_data(&server, "v");
    let cx_id = ingested[0]["cx_id"].as_str().unwrap();

    let result = call_ok(
        &server,
        8,
        "calyx.neighbors",
        json!({"vault": "v", "cx_id": cx_id, "k": 5}),
    );

    let neighbors = result["neighbors"].as_array().unwrap();
    assert!(!neighbors.is_empty());
    assert!(neighbors.len() <= 5);
    for item in neighbors {
        let score = item["score"].as_f64().unwrap();
        assert!((0.0..=1.0).contains(&score));
        assert!(item["cx_id"].as_str().unwrap().len() == 32);
    }
}

#[test]
fn empty_vault_search_returns_empty_hits_without_error() {
    let _env = TestEnv::new("empty");
    let server = server();
    call_ok(&server, 9, "calyx.create_vault", json!({"name": "v"}));

    let result = call_ok(
        &server,
        10,
        "calyx.search",
        json!({"vault": "v", "query": "alpha"}),
    );

    assert_eq!(result["hits"].as_array().unwrap().len(), 0);
}

#[test]
fn invalid_search_arguments_are_invalid_params() {
    let _env = TestEnv::new("invalid");
    let server = server();

    let zero_k = call_err(
        &server,
        11,
        "calyx.search",
        json!({"vault": "v", "query": "alpha", "k": 0}),
    );
    let bad_fusion = call_err(
        &server,
        12,
        "calyx.search",
        json!({"vault": "v", "query": "alpha", "fusion": "unknown"}),
    );

    assert_eq!(zero_k.code, -32602);
    assert_eq!(bad_fusion.code, -32602);
}

#[test]
fn in_region_guard_requires_calibration() {
    let _env = TestEnv::new("guard");
    let server = server();
    vault_with_algorithmic_data(&server, "v");

    let error = call_err(
        &server,
        13,
        "calyx.search",
        json!({"vault": "v", "query": "alpha", "guard": "in_region"}),
    );

    assert_eq!(error.code, -32000);
    assert_eq!(error.data.unwrap()["calyx_code"], "CALYX_GUARD_PROVISIONAL");
}
