use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::{Value, json};

const TEST_MASTER_KEY_HEX: &str =
    "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff";

pub struct TestRoot {
    pub path: PathBuf,
}

impl TestRoot {
    pub fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "calyx-leapable-storage-{name}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("create data root");
        Self {
            path: path.canonicalize().expect("canonical data root"),
        }
    }
}

impl Drop for TestRoot {
    fn drop(&mut self) {
        if self.path.starts_with(std::env::temp_dir()) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}

pub fn request(id: u64, method: &str, params: Value) -> String {
    let mut out = serde_json::to_string(&json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": method,
        "params": params
    }))
    .unwrap();
    out.push('\n');
    out
}

pub fn run_engine(input: &str, root: &Path) -> (String, String, bool) {
    let exe = env!("CARGO_BIN_EXE_calyx-leapable");
    let mut child = Command::new(exe)
        .arg("--data-dir")
        .arg(root)
        .env("CALYX_LEAPABLE_MASTER_KEY_HEX", TEST_MASTER_KEY_HEX)
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

pub fn json_lines(stdout: &str) -> Vec<Value> {
    stdout
        .lines()
        .map(|line| serde_json::from_str(line).expect("stdout line is JSON-RPC"))
        .collect()
}

pub fn storage_dir(root: &Path, vault_ref: &str) -> PathBuf {
    root.join(format!("{vault_ref}.calyx"))
}

pub fn wal_files(vault_root: &Path) -> Vec<String> {
    fs::read_dir(vault_root.join("wal"))
        .expect("read wal dir")
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            name.ends_with(".wal").then_some(name)
        })
        .collect()
}

pub fn assert_no_json_on_stderr(stderr: &str) {
    assert!(
        !stderr.contains('{'),
        "stderr must not contain protocol JSON: {stderr:?}"
    );
}

pub fn assert_calyx_code(response: &Value, code: &str) {
    assert_eq!(response["error"]["data"]["calyx_code"], code, "{response}");
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
