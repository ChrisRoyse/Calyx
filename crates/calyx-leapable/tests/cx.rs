//! End-to-end stdio tests for Leapable constellation CRUD RPCs.

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
    assert!(
        !storage_dir(root.path(), "cxlife").exists(),
        "before absent"
    );

    let setup = [
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
    ]
    .concat();

    let (stdout, stderr, ok) = run_engine(&setup, root.path());
    let responses = json_lines(&stdout);
    assert_eq!(responses.len(), 2);
    for response in &responses {
        assert!(response.get("error").is_none(), "{response}");
    }
    let first_id = responses[1]["result"]["items"][0]["cx_id"]
        .as_str()
        .unwrap()
        .to_string();
    let second_id = responses[1]["result"]["items"][1]["cx_id"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(
        responses[1]["result"]["items"][0]["input"]["len"],
        first_text.len()
    );
    assert_eq!(
        responses[1]["result"]["items"][1]["input"]["len"],
        second_text.len()
    );
    assert!(
        !stderr.contains('{'),
        "stderr must not contain protocol JSON"
    );
    assert!(ok);

    let input = [
        request(
            3,
            "vault.open",
            json!({"vault_ref": "cxlife", "ts": 1_785_500_002_000_u64}),
        ),
        request(
            4,
            "cx.get",
            json!({
                "vault_ref": "cxlife",
                "ts": 1_785_500_002_500_u64,
                "cx_id": first_id.clone(),
                "include_input": true
            }),
        ),
        request(
            5,
            "cx.put",
            json!({
                "vault_ref": "cxlife",
                "ts": 1_785_500_003_000_u64,
                "panel_version": 7,
                "modality": "text",
                "input": {"text": first_text},
                "metadata": {"chunk_id": "chunk-alpha-repeat"},
                "scalars": {"tokens": 3.0}
            }),
        ),
        request(
            6,
            "cx.anchor",
            json!({
                "vault_ref": "cxlife",
                "ts": 1_785_500_004_000_u64,
                "cx_id": second_id.clone(),
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
            7,
            "cx.delete",
            json!({"vault_ref": "cxlife", "ts": 1_785_500_005_000_u64, "cx_id": first_id.clone()}),
        ),
        request(
            8,
            "cx.scan",
            json!({"vault_ref": "cxlife", "ts": 1_785_500_006_000_u64, "limit": 10}),
        ),
    ]
    .concat();

    let (stdout, stderr, ok) = run_engine(&input, root.path());
    let responses = json_lines(&stdout);
    assert_eq!(responses.len(), 6);
    for response in &responses {
        assert!(response.get("error").is_none(), "{response}");
    }
    assert_eq!(
        responses[1]["result"]["item"]["input_hex"],
        hex(first_text.as_bytes())
    );
    assert_eq!(responses[1]["result"]["item"]["input_text"], first_text);
    assert_eq!(responses[2]["result"]["cx_id"], first_id);
    assert_eq!(responses[2]["result"]["deduped"], true);
    assert_eq!(responses[2]["result"]["recurrence_occurrence"], 0);
    assert_eq!(responses[3]["result"]["anchor_count"], 1);
    assert_eq!(responses[4]["result"]["erase"]["records_deleted"], 1);
    assert_eq!(responses[4]["result"]["tombstones_truncated"], false);
    assert_eq!(
        responses[4]["result"]["erase"]["tombstone"]["records_deleted"],
        1
    );
    assert_tombstone_for(&responses[4]["result"]["tombstones"], &first_id);
    let scan = &responses[5]["result"];
    assert_eq!(scan["items"].as_array().unwrap().len(), 1);
    assert_eq!(scan["items"][0]["cx_id"], second_id);
    assert!(scan["items"][0].get("input_hex").is_none());
    assert_eq!(scan["tombstones_truncated"], false);
    assert_tombstone_for(&scan["tombstones"], &first_id);
    assert!(
        !stderr.contains('{'),
        "stderr must not contain protocol JSON"
    );
    assert!(ok);

    let vault = storage_dir(root.path(), "cxlife");
    assert_eq!(fs::read(vault.join("salt")).expect("salt file").len(), 32);
    assert!(vault.join("cf/base").exists());
    assert!(!vault.join("cf/slot_00").exists());
    assert!(!vault.join("cf/slot_01").exists());
    assert!(!vault.join("cf/slot_02").exists());
    assert!(vault.join("cf/leapable").exists());
    assert!(vault.join("cf/ledger").exists());
    assert!(!wal_files(&vault).is_empty(), "WAL bytes were written");
}

#[test]
fn duplicate_put_anchor_merge_is_idempotent_and_conflicts_fail_closed() {
    let root = TestRoot::new("anchor-merge");
    let text = "anchored duplicate chunk";
    let mut requests = vec![
        request(
            1,
            "vault.create",
            json!({"vault_ref": "anchors", "ts": 1_785_510_000_000_u64}),
        ),
        request(
            2,
            "cx.put",
            anchored_item(1_785_510_001_000_u64, text, true),
        ),
    ];
    for index in 0..100_u64 {
        requests.push(request(
            3 + index,
            "cx.put",
            anchored_item(1_785_510_002_000_u64 + index, text, true),
        ));
    }
    let setup = requests.concat();
    let (stdout, stderr, ok) = run_engine(&setup, root.path());
    let mut responses = json_lines(&stdout);
    assert_eq!(responses.len(), 102);
    for response in &responses {
        assert!(response.get("error").is_none(), "{response}");
    }
    assert!(ok);
    assert!(
        !stderr.contains('{'),
        "stderr must not contain protocol JSON"
    );
    let cx_id = responses[1]["result"]["cx_id"]
        .as_str()
        .unwrap()
        .to_string();

    let verify = [
        request(
            103,
            "vault.open",
            json!({"vault_ref": "anchors", "ts": 1_785_510_200_000_u64}),
        ),
        request(
            104,
            "cx.get",
            json!({"vault_ref": "anchors", "ts": 1_785_510_201_000_u64, "cx_id": cx_id}),
        ),
        request(
            105,
            "cx.put",
            anchored_item(1_785_510_202_000_u64, text, false),
        ),
        request(
            106,
            "cx.get",
            json!({"vault_ref": "anchors", "ts": 1_785_510_203_000_u64, "cx_id": responses[1]["result"]["cx_id"]}),
        ),
    ]
    .concat();
    let (stdout, stderr, ok) = run_engine(&verify, root.path());
    responses = json_lines(&stdout);
    assert_eq!(responses.len(), 4);
    assert!(responses[0].get("error").is_none(), "{responses:?}");
    assert!(responses[1].get("error").is_none(), "{responses:?}");
    let anchors = responses[1]["result"]["item"]["constellation"]["anchors"]
        .as_array()
        .unwrap();
    assert_eq!(anchors.len(), 3, "{anchors:?}");
    let conflict = responses[2]["error"]["data"].clone();
    assert_eq!(conflict["calyx_code"], "CALYX_LEAPABLE_ANCHOR_CONFLICT");
    assert_eq!(conflict["anchor_kind"], "test_pass");
    assert_eq!(conflict["existing_value"]["bool"], true);
    assert_eq!(conflict["incoming_value"]["bool"], false);
    let after_conflict = responses[3]["result"]["item"]["constellation"]["anchors"]
        .as_array()
        .unwrap();
    assert_eq!(after_conflict.len(), 3);
    assert!(ok);
    assert!(
        !stderr.contains('{'),
        "stderr must not contain protocol JSON"
    );
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
    assert_calyx_code(&responses[5], "CALYX_LEAPABLE_UNSERVED_CAPABILITY");
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
        "scalars": {"tokens": 3.0},
        "metadata": {"chunk_id": chunk_id, "document_id": "doc-1"}
    })
}

fn anchored_item(ts: u64, text: &str, test_pass: bool) -> Value {
    json!({
        "vault_ref": "anchors",
        "ts": ts,
        "panel_version": 7,
        "modality": "text",
        "input": {"text": text},
        "metadata": {"document_id": "anchor-doc"},
        "anchors": [
            {
                "kind": "test_pass",
                "value": {"bool": test_pass},
                "source": "cx-anchor-test",
                "observed_at": ts,
                "confidence": 1.0
            },
            {
                "kind": "reward",
                "value": {"number": 1.0},
                "source": "cx-anchor-test",
                "observed_at": ts,
                "confidence": 1.0
            },
            {
                "kind": {"label": "reviewed"},
                "value": {"text": "accepted"},
                "source": "cx-anchor-test",
                "observed_at": ts,
                "confidence": 1.0
            }
        ]
    })
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
