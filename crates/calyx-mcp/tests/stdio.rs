//! End-to-end FSV for the `calyx-mcp` stdio loop.
//!
//! These tests drive the *real compiled binary* (`CARGO_BIN_EXE_calyx-mcp`),
//! feeding it newline-delimited JSON-RPC on stdin and asserting on the raw
//! stdout/stderr bytes and the process exit code: the wire is the source of
//! truth, not any in-process return value.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::{Value, json};

const EXPECTED_TOOLS: &[&str] = &[
    "calyx.abundance",
    "calyx.add_lens",
    "calyx.agree",
    "calyx.anchor",
    "calyx.anneal.status",
    "calyx.answer_trace",
    "calyx.bits",
    "calyx.create_vault",
    "calyx.define",
    "calyx.disagree",
    "calyx.guard.calibrate",
    "calyx.guard.check",
    "calyx.guard_generate",
    "calyx.ingest",
    "calyx.ingest_media",
    "calyx.kernel",
    "calyx.kernel_answer",
    "calyx.list_panel",
    "calyx.measure",
    "calyx.neighbors",
    "calyx.park_lens",
    "calyx.profile_lens",
    "calyx.propose_lens",
    "calyx.provenance",
    "calyx.reproduce",
    "calyx.retire_lens",
    "calyx.search",
    "calyx.search_skill",
    "calyx.skills",
    "calyx.traverse",
    "calyx.verify_chain",
];

struct TestHome {
    path: PathBuf,
}

impl TestHome {
    fn new(name: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("calyx-mcp-stdio-{name}-{}", std::process::id()));
        if path.exists() {
            fs::remove_dir_all(&path).expect("remove stale test home");
        }
        fs::create_dir_all(&path).expect("create test home");
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TestHome {
    fn drop(&mut self) {
        if self.path.starts_with(std::env::temp_dir()) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}

/// Runs the binary with `input` on stdin; returns `(stdout, stderr, exit_ok)`.
fn run_mcp(input: &str) -> (String, String, bool) {
    run_mcp_with_home(input, None)
}

fn run_mcp_with_home(input: &str, home: Option<&Path>) -> (String, String, bool) {
    let exe = env!("CARGO_BIN_EXE_calyx-mcp");
    let mut command = Command::new(exe);
    if let Some(home) = home {
        command.env("CALYX_HOME", home);
    }
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn calyx-mcp");

    child
        .stdin
        .take()
        .expect("stdin handle")
        .write_all(input.as_bytes())
        .expect("write stdin");
    // stdin dropped here -> EOF to the child.

    let output = child.wait_with_output().expect("wait for calyx-mcp");
    (
        String::from_utf8(output.stdout).expect("utf8 stdout"),
        String::from_utf8(output.stderr).expect("utf8 stderr"),
        output.status.success(),
    )
}

fn startup_stderr() -> String {
    format!("calyx-mcp: registered {} tools\n", EXPECTED_TOOLS.len())
}

fn assert_startup_only(stderr: &str) {
    assert_eq!(stderr, startup_stderr());
}

fn assert_no_json_on_stderr(stderr: &str) {
    assert!(
        !stderr.contains('{'),
        "stderr must not contain JSON-RPC bytes: {stderr:?}"
    );
}

fn line(value: Value) -> String {
    let mut out = serde_json::to_string(&value).unwrap();
    out.push('\n');
    out
}

fn tool_call(id: u64, name: &str, arguments: Value) -> String {
    line(json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "tools/call",
        "params": { "name": name, "arguments": arguments }
    }))
}

fn json_lines(stdout: &str) -> Vec<Value> {
    stdout
        .lines()
        .map(|line| serde_json::from_str(line).expect("stdout line is JSON-RPC"))
        .collect()
}

fn assert_no_error(response: &Value) {
    assert!(
        response.get("error").is_none(),
        "unexpected JSON-RPC error: {response}"
    );
}

fn tool_payload(response: &Value) -> Value {
    assert_no_error(response);
    let text = response["result"]["content"][0]["text"]
        .as_str()
        .expect("tool result text payload");
    serde_json::from_str(text).expect("tool text payload is JSON")
}

#[test]
fn tools_list_returns_registered_vault_tools_and_clean_exit() {
    let (stdout, stderr, ok) =
        run_mcp("{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/list\",\"params\":{}}\n");

    let response: Value = serde_json::from_str(stdout.trim()).expect("stdout is one JSON-RPC line");
    assert_eq!(response["jsonrpc"], "2.0");
    assert_eq!(response["id"], 1);
    let tools = response["result"]["tools"].as_array().unwrap();
    assert_eq!(tools.len(), EXPECTED_TOOLS.len());
    let names = tools
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(names, EXPECTED_TOOLS);
    for tool in tools {
        assert!(!tool["description"].as_str().unwrap().is_empty());
        assert!(!tool["use_when"].as_str().unwrap().is_empty());
        assert!(tool["inputSchema"].is_object());
    }
    assert!(response.get("error").is_none());
    assert_startup_only(&stderr);
    assert!(ok, "clean exit on EOF");
}

#[test]
fn response_id_mirrors_request_id_for_string_and_int() {
    let input = "{\"jsonrpc\":\"2.0\",\"id\":\"alpha\",\"method\":\"initialize\"}\n\
                 {\"jsonrpc\":\"2.0\",\"id\":42,\"method\":\"tools/list\"}\n";
    let (stdout, stderr, ok) = run_mcp(input);

    let lines = json_lines(&stdout);
    assert_eq!(lines.len(), 2, "one response per request");
    let first = &lines[0];
    let second = &lines[1];
    assert_eq!(first["id"], "alpha");
    assert_eq!(first["result"]["protocolVersion"], "2024-11-05");
    assert_eq!(first["result"]["capabilities"], json!({"tools": {}}));
    assert_eq!(first["result"]["serverInfo"]["name"], "calyx-mcp");
    assert_eq!(
        first["result"]["serverInfo"]["version"],
        env!("CARGO_PKG_VERSION")
    );
    assert_eq!(second["id"], 42);
    assert_startup_only(&stderr);
    assert!(ok);
}

#[test]
fn unknown_method_returns_minus_32601() {
    let (stdout, stderr, ok) =
        run_mcp("{\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"does/not/exist\"}\n");
    let response: Value = serde_json::from_str(stdout.trim()).unwrap();
    assert_eq!(response["error"]["code"], -32601);
    assert_eq!(response["id"], 3);
    assert_startup_only(&stderr);
    assert!(ok);
}

#[test]
fn unknown_tool_call_returns_minus_32601_with_startup_only_stderr() {
    let (stdout, stderr, ok) = run_mcp(
        "{\"jsonrpc\":\"2.0\",\"id\":4,\"method\":\"tools/call\",\"params\":{\"name\":\"ghost\"}}\n",
    );
    let response: Value = serde_json::from_str(stdout.trim()).unwrap();
    assert_eq!(response["error"]["code"], -32601);
    assert_startup_only(&stderr);
    assert!(ok);
}

#[test]
fn malformed_line_logs_to_stderr_and_next_line_still_processed() {
    let input = "this is not json\n\
                 {\"jsonrpc\":\"2.0\",\"id\":5,\"method\":\"tools/list\"}\n";
    let (stdout, stderr, ok) = run_mcp(input);

    // Exactly one response: the malformed line produced no stdout, only stderr.
    let lines = json_lines(&stdout);
    assert_eq!(
        lines.len(),
        1,
        "malformed line must not emit a stdout response"
    );
    assert_eq!(lines[0]["id"], 5);
    assert!(
        stderr.starts_with(&startup_stderr()),
        "startup line should be first, got: {stderr:?}"
    );
    assert!(
        stderr.contains("CALYX_MCP_JSONRPC_INVALID"),
        "malformed line is reported on stderr, got: {stderr:?}"
    );
    assert_no_json_on_stderr(&stderr);
    assert!(ok, "server survives a malformed line and exits cleanly");
}

#[test]
fn empty_lines_are_ignored() {
    let input = "\n   \n{\"jsonrpc\":\"2.0\",\"id\":6,\"method\":\"tools/list\"}\n\n";
    let (stdout, stderr, ok) = run_mcp(input);
    let lines = json_lines(&stdout);
    assert_eq!(lines.len(), 1, "blank lines emit nothing");
    assert_startup_only(&stderr);
    assert!(ok);
}

#[test]
fn notification_without_id_gets_no_response() {
    // A request with no `id` is a notification -> no reply, but the following
    // request with an id still gets answered.
    let input = "{\"jsonrpc\":\"2.0\",\"method\":\"initialize\"}\n\
                 {\"jsonrpc\":\"2.0\",\"id\":7,\"method\":\"tools/list\"}\n";
    let (stdout, stderr, ok) = run_mcp(input);
    let lines = json_lines(&stdout);
    assert_eq!(lines.len(), 1, "notification must not produce a response");
    assert_eq!(lines[0]["id"], 7);
    assert_startup_only(&stderr);
    assert!(ok);
}

#[test]
fn null_id_notification_gets_no_response() {
    let input = "{\"jsonrpc\":\"2.0\",\"id\":null,\"method\":\"initialize\"}\n\
                 {\"jsonrpc\":\"2.0\",\"id\":8,\"method\":\"tools/list\"}\n";
    let (stdout, stderr, ok) = run_mcp(input);
    let lines = json_lines(&stdout);
    assert_eq!(lines.len(), 1, "id:null notification must not respond");
    assert_eq!(lines[0]["id"], 8);
    assert_startup_only(&stderr);
    assert!(ok);
}

#[test]
fn immediate_eof_exits_cleanly_with_startup_log_only() {
    let (stdout, stderr, ok) = run_mcp("");
    assert_eq!(stdout, "");
    assert_startup_only(&stderr);
    assert!(ok, "EOF with no input -> exit 0");
}

#[test]
fn full_mcp_workflow_smoke_returns_provenance_and_remediation() {
    let home = TestHome::new("workflow");
    let vault = "mcp-stdio-workflow";
    let first_input = [
        line(json!({"jsonrpc": "2.0", "id": 1, "method": "initialize"})),
        line(json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"})),
        tool_call(3, "calyx.create_vault", json!({"name": vault})),
        tool_call(
            4,
            "calyx.add_lens",
            json!({"vault": vault, "name": "byte_axis", "runtime": "algorithmic"}),
        ),
        tool_call(
            5,
            "calyx.ingest",
            json!({"vault": vault, "input": "Why does X fail under load?"}),
        ),
    ]
    .concat();
    let (stdout, stderr, ok) = run_mcp_with_home(&first_input, Some(home.path()));
    assert_startup_only(&stderr);
    assert!(ok);
    let first = json_lines(&stdout);
    assert_eq!(first.len(), 5);
    for response in &first {
        assert_no_error(response);
    }
    let cx_id = tool_payload(&first[4])["cx_id"]
        .as_str()
        .unwrap()
        .to_string();

    let second_input = [
        tool_call(
            6,
            "calyx.anchor",
            json!({"vault": vault, "cx_id": cx_id, "kind": "test_pass", "value": true}),
        ),
        tool_call(
            7,
            "calyx.search",
            json!({"vault": vault, "query": "fail under load"}),
        ),
        tool_call(
            8,
            "calyx.bits",
            json!({"vault": vault, "anchor": "test_pass"}),
        ),
        tool_call(
            9,
            "calyx.provenance",
            json!({"vault": vault, "cx_id": cx_id}),
        ),
    ]
    .concat();
    let (stdout, stderr, ok) = run_mcp_with_home(&second_input, Some(home.path()));
    assert_startup_only(&stderr);
    assert!(ok);
    let second = json_lines(&stdout);
    assert_eq!(second.len(), 4);

    let anchor = tool_payload(&second[0]);
    assert_eq!(anchor["status"], "anchored");
    let search = tool_payload(&second[1]);
    let hit = &search["hits"].as_array().unwrap()[0];
    assert_eq!(hit["cx_id"], cx_id);
    assert!(hit["provenance"]["ledger_seq"].as_u64().unwrap() > 0);
    let bits = &second[2]["error"];
    assert_eq!(
        bits["data"]["calyx_code"],
        "CALYX_ASSAY_INSUFFICIENT_SAMPLES"
    );
    assert_eq!(bits["data"]["remediation"], "anchor ≥50 outcomes first");
    let provenance = tool_payload(&second[3]);
    assert_eq!(provenance["cx_id"], cx_id);
    assert!(provenance["ledger_chain_hash"].as_str().unwrap().len() >= 32);
}

#[test]
fn repeated_ingest_is_idempotent_over_stdio() {
    let home = TestHome::new("idempotent");
    let vault = "mcp-stdio-idempotent";
    let input = [
        tool_call(1, "calyx.create_vault", json!({"name": vault})),
        tool_call(
            2,
            "calyx.add_lens",
            json!({"vault": vault, "name": "byte_axis", "runtime": "algorithmic"}),
        ),
        tool_call(
            3,
            "calyx.ingest",
            json!({"vault": vault, "input": "same text"}),
        ),
        tool_call(
            4,
            "calyx.ingest",
            json!({"vault": vault, "input": "same text"}),
        ),
    ]
    .concat();
    let (stdout, stderr, ok) = run_mcp_with_home(&input, Some(home.path()));
    assert_startup_only(&stderr);
    assert!(ok);
    let responses = json_lines(&stdout);
    assert_eq!(responses.len(), 4);
    let first = tool_payload(&responses[2]);
    let second = tool_payload(&responses[3]);
    assert_eq!(first["cx_id"], second["cx_id"]);
    assert_eq!(first["new"], true);
    assert_eq!(second["new"], false);
}

#[test]
fn kernel_answer_ungrounded_error_has_exact_remediation() {
    let home = TestHome::new("kernel-answer");
    let vault = "mcp-stdio-kernel-answer";
    let input = [
        tool_call(1, "calyx.create_vault", json!({"name": vault})),
        tool_call(
            2,
            "calyx.add_lens",
            json!({"vault": vault, "name": "byte_axis", "runtime": "algorithmic"}),
        ),
        tool_call(
            3,
            "calyx.ingest",
            json!({"vault": vault, "input": "ungrounded"}),
        ),
        tool_call(
            4,
            "calyx.kernel_answer",
            json!({"vault": vault, "query": "ungrounded"}),
        ),
    ]
    .concat();
    let (stdout, stderr, ok) = run_mcp_with_home(&input, Some(home.path()));
    assert_startup_only(&stderr);
    assert!(ok);
    let responses = json_lines(&stdout);
    assert_eq!(responses.len(), 4);
    let error = &responses[3]["error"];
    assert_eq!(error["data"]["calyx_code"], "CALYX_KERNEL_UNGROUNDED");
    assert_eq!(error["data"]["remediation"], "add anchors (grounding_gaps)");
}
