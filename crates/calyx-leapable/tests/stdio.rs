//! End-to-end stdio tests for the `calyx-leapable` engine process.
//!
//! These drive the real compiled binary and assert on raw stdout/stderr plus
//! durable vault bytes under the sidecar-provided data root.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::{Value, json};

const TEST_MASTER_KEY_HEX: &str =
    "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff";

struct TestRoot {
    path: PathBuf,
}

impl TestRoot {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "calyx-leapable-stdio-{name}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("create data root");
        Self {
            path: path.canonicalize().expect("canonical data root"),
        }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TestRoot {
    fn drop(&mut self) {
        if self.path.starts_with(std::env::temp_dir()) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}

fn line(value: Value) -> String {
    let mut out = serde_json::to_string(&value).unwrap();
    out.push('\n');
    out
}

fn request(id: u64, method: &str, params: Value) -> String {
    line(json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": method,
        "params": params
    }))
}

fn run_engine(input: &str, root: &Path, panic_probe: bool) -> (String, String, bool) {
    let exe = env!("CARGO_BIN_EXE_calyx-leapable");
    let mut command = Command::new(exe);
    command.arg("--data-dir").arg(root);
    command.env("CALYX_LEAPABLE_MASTER_KEY_HEX", TEST_MASTER_KEY_HEX);
    if panic_probe {
        command.env("CALYX_LEAPABLE_ENABLE_PANIC_PROBE", "1");
    }
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn calyx-leapable");
    child
        .stdin
        .take()
        .expect("stdin handle")
        .write_all(input.as_bytes())
        .expect("write stdin");
    let output = child.wait_with_output().expect("wait for calyx-leapable");
    (
        String::from_utf8(output.stdout).expect("utf8 stdout"),
        String::from_utf8(output.stderr).expect("utf8 stderr"),
        output.status.success(),
    )
}

fn json_lines(stdout: &str) -> Vec<Value> {
    stdout
        .lines()
        .map(|line| serde_json::from_str(line).expect("stdout line is JSON-RPC"))
        .collect()
}

fn assert_no_json_on_stderr(stderr: &str) {
    assert!(
        !stderr.contains('{'),
        "stderr must not contain protocol JSON: {stderr:?}"
    );
}

#[test]
fn multiplexes_two_vaults_and_survives_panic_request() {
    let root = TestRoot::new("multiplex-panic");
    assert!(!root.path().join("alpha").exists(), "before: alpha absent");
    assert!(!root.path().join("beta").exists(), "before: beta absent");

    let input = [
        request(1, "engine.info", json!({})),
        request(
            2,
            "vault.open",
            json!({"vault_ref": "alpha", "ts": 1785500000_u64}),
        ),
        request(
            3,
            "vault.open",
            json!({"vault_ref": "beta", "ts": 1785500010_u64}),
        ),
        request(
            4,
            "vault.stat",
            json!({"vault_ref": "alpha", "ts": 1785500020_u64}),
        ),
        request(5, "engine.panic_probe", json!({})),
        request(
            6,
            "vault.stat",
            json!({"vault_ref": "beta", "ts": 1785500030_u64}),
        ),
    ]
    .concat();
    let (stdout, stderr, ok) = run_engine(&input, root.path(), true);
    let responses = json_lines(&stdout);
    assert_eq!(responses.len(), 6);
    for response in responses.iter().take(4) {
        assert!(response.get("error").is_none(), "{response}");
    }
    assert_eq!(responses[4]["error"]["code"], -32603);
    assert_eq!(responses[4]["error"]["message"], "internal server error");
    assert!(
        responses[5].get("error").is_none(),
        "post-panic request served"
    );
    assert_eq!(responses[5]["result"]["vault_ref"], "beta");
    assert_eq!(responses[5]["result"]["last_ts"], 1785500030_u64);
    assert!(stderr.contains("calyx-leapable: stdio engine ready"));
    assert!(stderr.contains("request panic isolated"));
    assert_no_json_on_stderr(&stderr);
    assert!(ok, "engine exits cleanly on EOF after panic isolation");

    assert!(root.path().join("alpha").join("cf").join("base").exists());
    assert!(root.path().join("beta").join("cf").join("base").exists());
    assert!(!wal_files(&root.path().join("alpha")).is_empty());
    assert!(!wal_files(&root.path().join("beta")).is_empty());
}

#[test]
fn malformed_line_logs_to_stderr_and_next_request_is_served() {
    let root = TestRoot::new("malformed");
    let input = [
        "not-json\n".to_string(),
        request(1, "engine.info", json!({})),
    ]
    .concat();
    let (stdout, stderr, ok) = run_engine(&input, root.path(), false);
    let responses = json_lines(&stdout);
    assert_eq!(responses.len(), 1);
    assert_eq!(responses[0]["id"], 1);
    assert!(stderr.contains("CALYX_MCP_JSONRPC_INVALID"));
    assert_no_json_on_stderr(&stderr);
    assert!(ok);
}

#[test]
fn unsafe_vault_ref_is_rejected_without_creating_escape_dir() {
    let root = TestRoot::new("unsafe-ref");
    let input = request(
        1,
        "vault.open",
        json!({"vault_ref": "../escape", "ts": 1785500000_u64}),
    );
    let (stdout, stderr, ok) = run_engine(&input, root.path(), false);
    let responses = json_lines(&stdout);
    assert_eq!(responses.len(), 1);
    assert_eq!(
        responses[0]["error"]["data"]["calyx_code"],
        "CALYX_LEAPABLE_PATH_INVALID"
    );
    assert!(!root.path().join("escape").exists());
    assert_no_json_on_stderr(&stderr);
    assert!(ok);
}

#[test]
fn notification_gets_no_response_but_following_request_does() {
    let root = TestRoot::new("notification");
    let input = [
        line(json!({"jsonrpc": "2.0", "method": "engine.info", "params": {}})),
        request(1, "engine.info", json!({})),
    ]
    .concat();
    let (stdout, stderr, ok) = run_engine(&input, root.path(), false);
    let responses = json_lines(&stdout);
    assert_eq!(responses.len(), 1);
    assert_eq!(responses[0]["id"], 1);
    assert_no_json_on_stderr(&stderr);
    assert!(ok);
}

fn wal_files(vault_root: &Path) -> Vec<String> {
    let mut files = fs::read_dir(vault_root.join("wal"))
        .expect("read wal dir")
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            name.ends_with(".wal").then_some(name)
        })
        .collect::<Vec<_>>();
    files.sort();
    files
}
