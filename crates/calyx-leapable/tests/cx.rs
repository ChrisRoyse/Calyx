//! End-to-end stdio tests for Leapable constellation CRUD RPCs.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use calyx_core::CxId;
use serde_json::{Value, json};

const TEST_MASTER_KEY_HEX: &str =
    "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff";

struct TestRoot {
    path: PathBuf,
}

impl TestRoot {
    fn new(name: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("calyx-leapable-cx-{name}-{}", std::process::id()));
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

fn request(id: u64, method: &str, params: Value) -> String {
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

fn run_engine(input: &str, root: &Path) -> (String, String, bool) {
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

fn json_lines(stdout: &str) -> Vec<Value> {
    stdout
        .lines()
        .map(|line| serde_json::from_str(line).expect("stdout line is JSON-RPC"))
        .collect()
}

#[test]
fn cx_put_batch_get_anchor_delete_and_scan_round_trip() {
    let root = TestRoot::new("round-trip");
    let first_text = "alpha known chunk";
    let second_text = "beta known chunk";
    let first_id = expected_cx_id("cxlife", first_text.as_bytes(), 7);
    let second_id = expected_cx_id("cxlife", second_text.as_bytes(), 7);
    assert!(
        !storage_dir(root.path(), "cxlife").exists(),
        "before absent"
    );

    let input = [
        request(
            1,
            "vault.create",
            json!({"vault_ref": "cxlife", "ts": 1_785_500_000_000_u64}),
        ),
        request(
            2,
            "cx.put_batch",
            json!({
                "vault_ref": "cxlife",
                "ts": 1_785_500_001_000_u64,
                "items": [put_item(first_text, "chunk-alpha"), put_item(second_text, "chunk-beta")]
            }),
        ),
        request(
            3,
            "cx.get",
            json!({"vault_ref": "cxlife", "ts": 1_785_500_002_000_u64, "cx_id": first_id.to_string()}),
        ),
        request(
            4,
            "cx.put",
            json!({
                "vault_ref": "cxlife",
                "ts": 1_785_500_003_000_u64,
                "panel_version": 7,
                "modality": "text",
                "input": {"text": first_text},
                "slots": [{"slot_id": 0, "vector": {"dense": {"dim": 3, "data": [0.1, 0.2, 0.3]}}}],
                "metadata": {"chunk_id": "chunk-alpha-repeat"},
                "scalars": {"tokens": 3.0}
            }),
        ),
        request(
            5,
            "cx.anchor",
            json!({
                "vault_ref": "cxlife",
                "ts": 1_785_500_004_000_u64,
                "cx_id": second_id.to_string(),
                "anchor": {
                    "kind": "test_pass",
                    "value": {"bool": true},
                    "source": "cx-stdio-test",
                    "observed_at": 1_785_500_004_000_u64,
                    "confidence": 1.0
                }
            }),
        ),
        request(
            6,
            "cx.delete",
            json!({"vault_ref": "cxlife", "ts": 1_785_500_005_000_u64, "cx_id": first_id.to_string()}),
        ),
        request(
            7,
            "cx.scan",
            json!({"vault_ref": "cxlife", "ts": 1_785_500_006_000_u64, "limit": 10}),
        ),
    ]
    .concat();

    let (stdout, stderr, ok) = run_engine(&input, root.path());
    let responses = json_lines(&stdout);
    assert_eq!(responses.len(), 7);
    for response in &responses {
        assert!(response.get("error").is_none(), "{response}");
    }
    assert_eq!(
        responses[1]["result"]["items"][0]["cx_id"],
        first_id.to_string()
    );
    assert_eq!(
        responses[1]["result"]["items"][1]["cx_id"],
        second_id.to_string()
    );
    assert_eq!(
        responses[2]["result"]["item"]["input_hex"],
        hex(first_text.as_bytes())
    );
    assert_eq!(responses[2]["result"]["item"]["input_text"], first_text);
    assert_eq!(responses[3]["result"]["cx_id"], first_id.to_string());
    assert_eq!(responses[3]["result"]["deduped"], true);
    assert_eq!(responses[3]["result"]["recurrence_occurrence"], 0);
    assert_eq!(responses[4]["result"]["anchor_count"], 1);
    assert_eq!(responses[5]["result"]["erase"]["records_deleted"], 1);
    assert_tombstone_for(&responses[5]["result"]["tombstones"], &first_id.to_string());
    let scan = &responses[6]["result"];
    assert_eq!(scan["items"].as_array().unwrap().len(), 1);
    assert_eq!(scan["items"][0]["cx_id"], second_id.to_string());
    assert_tombstone_for(&scan["tombstones"], &first_id.to_string());
    assert!(
        !stderr.contains('{'),
        "stderr must not contain protocol JSON"
    );
    assert!(ok);

    let vault = storage_dir(root.path(), "cxlife");
    assert!(vault.join("cf/base").exists());
    assert!(vault.join("cf/slot_00").exists());
    assert!(vault.join("cf/slot_01").exists());
    assert!(vault.join("cf/slot_02").exists());
    assert!(vault.join("cf/ledger").exists());
    assert!(!wal_files(&vault).is_empty(), "WAL bytes were written");
}

#[test]
fn cx_rpc_edges_fail_closed() {
    let root = TestRoot::new("edges");
    let input = [
        request(
            1,
            "cx.put",
            json!({
                "vault_ref": "ghost",
                "ts": 1_785_500_000_000_u64,
                "panel_version": 7,
                "modality": "text",
                "input": {"text": "unopened"}
            }),
        ),
        request(
            2,
            "vault.create",
            json!({"vault_ref": "edges", "ts": 1_785_500_001_000_u64}),
        ),
        request(
            3,
            "cx.put",
            json!({
                "vault_ref": "edges",
                "ts": 1_785_500_002_000_u64,
                "panel_version": 7,
                "modality": "text",
                "input": {"text": "ambiguous", "hex": "616d626967756f7573"}
            }),
        ),
        request(
            4,
            "cx.get",
            json!({"vault_ref": "edges", "ts": 1_785_500_003_000_u64, "cx_id": "not-a-cx-id"}),
        ),
        request(
            5,
            "cx.scan",
            json!({"vault_ref": "edges", "ts": 1_785_500_004_000_u64, "limit": 0}),
        ),
        request(
            6,
            "cx.put",
            json!({
                "vault_ref": "edges",
                "ts": 1_785_500_005_000_u64,
                "panel_version": 7,
                "modality": "text",
                "input": {"text": "bad vector"},
                "slots": [{"slot_id": 0, "vector": {"dense": {"dim": 3, "data": [1.0]}}}]
            }),
        ),
    ]
    .concat();
    let (stdout, stderr, ok) = run_engine(&input, root.path());
    let responses = json_lines(&stdout);
    assert_eq!(responses.len(), 6);
    assert_calyx_code(&responses[0], "CALYX_LEAPABLE_VAULT_NOT_OPEN");
    assert!(responses[1].get("error").is_none());
    assert_calyx_code(&responses[2], "CALYX_LEAPABLE_CX_INPUT_INVALID");
    assert_calyx_code(&responses[3], "CALYX_LEAPABLE_CX_ID_INVALID");
    assert_calyx_code(&responses[4], "CALYX_LEAPABLE_CX_SCAN_LIMIT_INVALID");
    assert_calyx_code(&responses[5], "CALYX_RECORD_SCHEMA_VIOLATION");
    assert!(
        !stderr.contains('{'),
        "stderr must not contain protocol JSON"
    );
    assert!(ok);
}

fn put_item(text: &str, chunk_id: &str) -> Value {
    json!({
        "panel_version": 7,
        "modality": "text",
        "input": {"text": text, "pointer": format!("leapable://{chunk_id}")},
        "slots": [
            {"slot_id": 0, "vector": {"dense": {"dim": 3, "data": [0.1, 0.2, 0.3]}}},
            {"slot_id": 1, "vector": {"sparse": {"dim": 16, "entries": [{"idx": 2, "val": 1.25}]}}},
            {"slot_id": 2, "vector": {"multi": {"token_dim": 2, "tokens": [[0.5, 0.6], [0.7, 0.8]]}}}
        ],
        "scalars": {"tokens": 3.0},
        "metadata": {"chunk_id": chunk_id, "document_id": "doc-1"}
    })
}

fn expected_cx_id(vault_ref: &str, input: &[u8], panel_version: u32) -> CxId {
    CxId::from_input(input, panel_version, &salt_for(vault_ref))
}

fn salt_for(vault_ref: &str) -> Vec<u8> {
    calyx_core::content_address([
        b"calyx-leapable-vault-salt".as_slice(),
        vault_ref.as_bytes(),
    ])
    .to_vec()
}

fn storage_dir(root: &Path, vault_ref: &str) -> PathBuf {
    root.join(format!("{vault_ref}.calyx"))
}

fn wal_files(vault_root: &Path) -> Vec<String> {
    fs::read_dir(vault_root.join("wal"))
        .expect("read wal dir")
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            name.ends_with(".wal").then_some(name)
        })
        .collect()
}

fn assert_calyx_code(response: &Value, code: &str) {
    assert_eq!(response["error"]["data"]["calyx_code"], code, "{response}");
}

fn assert_tombstone_for(tombstones: &Value, cx_id: &str) {
    let tombstones = tombstones.as_array().expect("tombstones array");
    assert!(
        tombstones
            .iter()
            .any(|value| value["compact"]["c"].as_str() == Some(cx_id)),
        "missing tombstone for {cx_id}: {tombstones:?}"
    );
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
