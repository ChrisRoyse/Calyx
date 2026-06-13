//! End-to-end FSV for the `calyx-mcp` stdio loop.
//!
//! These tests drive the *real compiled binary* (`CARGO_BIN_EXE_calyx-mcp`),
//! feeding it newline-delimited JSON-RPC on stdin and asserting on the raw
//! stdout/stderr bytes and the process exit code — the wire is the source of
//! truth, not any in-process return value.

use std::io::Write;
use std::process::{Command, Stdio};

/// Runs the binary with `input` on stdin; returns `(stdout, stderr, exit_ok)`.
fn run_mcp(input: &str) -> (String, String, bool) {
    let exe = env!("CARGO_BIN_EXE_calyx-mcp");
    let mut child = Command::new(exe)
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
    // stdin dropped here → EOF to the child.

    let output = child.wait_with_output().expect("wait for calyx-mcp");
    (
        String::from_utf8(output.stdout).expect("utf8 stdout"),
        String::from_utf8(output.stderr).expect("utf8 stderr"),
        output.status.success(),
    )
}

#[test]
fn tools_list_returns_empty_array_and_clean_exit() {
    let (stdout, stderr, ok) =
        run_mcp("{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/list\",\"params\":{}}\n");

    let response: serde_json::Value =
        serde_json::from_str(stdout.trim()).expect("stdout is one JSON-RPC line");
    assert_eq!(response["jsonrpc"], "2.0");
    assert_eq!(response["id"], 1);
    assert_eq!(response["result"]["tools"], serde_json::json!([]));
    assert!(response.get("error").is_none());
    // Valid request → nothing leaks to stderr; process exits 0.
    assert_eq!(stderr, "", "stderr must be empty for a valid request");
    assert!(ok, "clean exit on EOF");
}

#[test]
fn response_id_mirrors_request_id_for_string_and_int() {
    let input = "{\"jsonrpc\":\"2.0\",\"id\":\"alpha\",\"method\":\"initialize\"}\n\
                 {\"jsonrpc\":\"2.0\",\"id\":42,\"method\":\"tools/list\"}\n";
    let (stdout, stderr, ok) = run_mcp(input);

    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 2, "one response per request");
    let first: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
    let second: serde_json::Value = serde_json::from_str(lines[1]).unwrap();
    assert_eq!(first["id"], "alpha");
    assert_eq!(first["result"]["serverInfo"]["name"], "calyx-mcp");
    assert_eq!(second["id"], 42);
    assert_eq!(stderr, "");
    assert!(ok);
}

#[test]
fn unknown_method_returns_minus_32601() {
    let (stdout, _stderr, _ok) =
        run_mcp("{\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"does/not/exist\"}\n");
    let response: serde_json::Value = serde_json::from_str(stdout.trim()).unwrap();
    assert_eq!(response["error"]["code"], -32601);
    assert_eq!(response["id"], 3);
}

#[test]
fn unknown_tool_call_returns_minus_32601_with_empty_stderr() {
    let (stdout, stderr, _ok) = run_mcp(
        "{\"jsonrpc\":\"2.0\",\"id\":4,\"method\":\"tools/call\",\"params\":{\"name\":\"ghost\"}}\n",
    );
    let response: serde_json::Value = serde_json::from_str(stdout.trim()).unwrap();
    assert_eq!(response["error"]["code"], -32601);
    assert_eq!(stderr, "", "a well-formed request must not log to stderr");
}

#[test]
fn malformed_line_logs_to_stderr_and_next_line_still_processed() {
    let input = "this is not json\n\
                 {\"jsonrpc\":\"2.0\",\"id\":5,\"method\":\"tools/list\"}\n";
    let (stdout, stderr, ok) = run_mcp(input);

    // Exactly one response: the malformed line produced no stdout, only stderr.
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 1, "malformed line must not emit a stdout response");
    let response: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(response["id"], 5);
    assert!(
        stderr.contains("CALYX_MCP_JSONRPC_INVALID"),
        "malformed line is reported on stderr, got: {stderr:?}"
    );
    assert!(ok, "server survives a malformed line and exits cleanly");
}

#[test]
fn empty_lines_are_ignored() {
    let input = "\n   \n{\"jsonrpc\":\"2.0\",\"id\":6,\"method\":\"tools/list\"}\n\n";
    let (stdout, stderr, ok) = run_mcp(input);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 1, "blank lines emit nothing");
    assert_eq!(stderr, "");
    assert!(ok);
}

#[test]
fn notification_without_id_gets_no_response() {
    // A request with no `id` is a notification → no reply, but the following
    // request with an id still gets answered.
    let input = "{\"jsonrpc\":\"2.0\",\"method\":\"initialize\"}\n\
                 {\"jsonrpc\":\"2.0\",\"id\":7,\"method\":\"tools/list\"}\n";
    let (stdout, _stderr, ok) = run_mcp(input);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 1, "notification must not produce a response");
    let response: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(response["id"], 7);
    assert!(ok);
}

#[test]
fn immediate_eof_exits_cleanly_with_no_output() {
    let (stdout, stderr, ok) = run_mcp("");
    assert_eq!(stdout, "");
    assert_eq!(stderr, "");
    assert!(ok, "EOF with no input → exit 0");
}
