//! At-rest security regression tests for the Leapable sidecar.

use std::fs;
use std::path::Path;

use serde_json::{Value, json};

use storage_support::{TestRoot, assert_no_json_on_stderr, json_lines, request, run_engine};

mod storage_support;

const MARKER: &str = "CALYX_LEAPABLE_AT_REST_MARKER_1352";

#[test]
fn vault_master_key_encrypts_cx_kv_and_blob_bytes_on_disk() {
    let root = TestRoot::new("security-at-rest");
    let vault_ref = "secure";
    let setup = [
        request(
            1,
            "vault.create",
            json!({"vault_ref": vault_ref, "ts": 1_785_700_000_000_u64}),
        ),
        request(
            2,
            "cx.put",
            json!({
                "vault_ref": vault_ref,
                "ts": 1_785_700_001_000_u64,
                "panel_version": 7,
                "modality": "text",
                "input": {"text": MARKER},
                "metadata": {"purpose": "security-at-rest"}
            }),
        ),
        request(
            3,
            "kv.set",
            json!({
                "vault_ref": vault_ref,
                "ts": 1_785_700_002_000_u64,
                "collection_name": "kvsec",
                "collection": {},
                "key": {"text": "marker-key"},
                "value": {"text": MARKER}
            }),
        ),
        request(
            4,
            "blob.put",
            json!({
                "vault_ref": vault_ref,
                "ts": 1_785_700_003_000_u64,
                "collection_name": "blobsec",
                "collection": {},
                "input": {"text": MARKER}
            }),
        ),
        request(
            5,
            "vault.close",
            json!({"vault_ref": vault_ref, "ts": 1_785_700_004_000_u64}),
        ),
    ]
    .concat();
    let (stdout, stderr, ok) = run_engine(&setup, &root.path);
    assert!(ok, "{stderr}");
    assert_no_json_on_stderr(&stderr);
    let responses = json_lines(&stdout);
    assert!(
        responses
            .iter()
            .all(|response| response.get("error").is_none())
    );
    assert_eq!(
        responses[0]["result"]["at_rest"]["value_encryption"],
        "aes-256-gcm"
    );
    let cx_id = responses[1]["result"]["cx_id"].as_str().unwrap();
    let blob_id = responses[3]["result"]["blob_id"].as_str().unwrap();
    assert_marker_absent(&root.path.join("secure.calyx"), MARKER.as_bytes());

    let readback = [
        request(
            6,
            "vault.open",
            json!({"vault_ref": vault_ref, "ts": 1_785_700_005_000_u64}),
        ),
        request(
            7,
            "cx.get",
            json!({
                "vault_ref": vault_ref,
                "ts": 1_785_700_006_000_u64,
                "cx_id": cx_id,
                "include_input": true
            }),
        ),
        request(
            8,
            "kv.get",
            json!({
                "vault_ref": vault_ref,
                "ts": 1_785_700_007_000_u64,
                "collection_name": "kvsec",
                "key": {"text": "marker-key"},
                "include_text": true
            }),
        ),
        request(
            9,
            "blob.get",
            json!({
                "vault_ref": vault_ref,
                "ts": 1_785_700_008_000_u64,
                "collection_name": "blobsec",
                "blob_id": blob_id,
                "include_data": true,
                "include_text": true
            }),
        ),
    ]
    .concat();
    let (stdout, stderr, ok) = run_engine(&readback, &root.path);
    assert!(ok, "{stderr}");
    let responses = json_lines(&stdout);
    assert!(
        responses
            .iter()
            .all(|response| response.get("error").is_none())
    );
    assert_eq!(responses[1]["result"]["item"]["input_text"], MARKER);
    assert_eq!(text_value(&responses[2]["result"]["value"]), MARKER);
    assert_eq!(text_value(&responses[3]["result"]["data"]), MARKER);
}

#[test]
fn cx_delete_shreds_live_key_and_rejects_later_encrypted_writes() {
    let root = TestRoot::new("security-shred");
    let vault_ref = "shred";
    let setup = [
        request(
            1,
            "vault.create",
            json!({"vault_ref": vault_ref, "ts": 1_785_710_000_000_u64}),
        ),
        request(
            2,
            "cx.put",
            json!({
                "vault_ref": vault_ref,
                "ts": 1_785_710_001_000_u64,
                "panel_version": 7,
                "modality": "text",
                "input": {"text": "delete-me-encrypted"}
            }),
        ),
        request(
            3,
            "vault.close",
            json!({"vault_ref": vault_ref, "ts": 1_785_710_002_000_u64}),
        ),
    ]
    .concat();
    let (stdout, stderr, ok) = run_engine(&setup, &root.path);
    assert!(ok, "{stderr}");
    let responses = json_lines(&stdout);
    let cx_id = responses[1]["result"]["cx_id"].as_str().unwrap();

    let delete_then_write = [
        request(
            4,
            "vault.open",
            json!({"vault_ref": vault_ref, "ts": 1_785_710_003_000_u64}),
        ),
        request(
            5,
            "cx.delete",
            json!({"vault_ref": vault_ref, "ts": 1_785_710_004_000_u64, "cx_id": cx_id}),
        ),
        request(
            6,
            "kv.set",
            json!({
                "vault_ref": vault_ref,
                "ts": 1_785_710_005_000_u64,
                "collection_name": "after_delete",
                "collection": {},
                "key": {"text": "must-fail"},
                "value": {"text": "must-not-seal"}
            }),
        ),
    ]
    .concat();
    let (stdout, stderr, ok) = run_engine(&delete_then_write, &root.path);
    assert!(ok, "{stderr}");
    let responses = json_lines(&stdout);
    assert!(responses[0].get("error").is_none(), "{}", responses[0]);
    assert!(responses[1].get("error").is_none(), "{}", responses[1]);
    assert_eq!(
        responses[2]["error"]["data"]["calyx_code"],
        "CALYX_VAULT_KEY_SHREDDED"
    );
}

fn assert_marker_absent(root: &Path, marker: &[u8]) {
    for path in file_paths(root) {
        let bytes = fs::read(&path).unwrap_or_else(|error| {
            panic!("read {}: {error}", path.display());
        });
        assert!(
            !contains_bytes(&bytes, marker),
            "plaintext marker found in {}",
            path.display()
        );
    }
}

fn file_paths(root: &Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    collect_files(root, &mut out);
    out
}

fn collect_files(root: &Path, out: &mut Vec<std::path::PathBuf>) {
    for entry in fs::read_dir(root).unwrap_or_else(|error| {
        panic!("read dir {}: {error}", root.display());
    }) {
        let path = entry.unwrap().path();
        if path.is_dir() {
            collect_files(&path, out);
        } else {
            out.push(path);
        }
    }
}

fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|candidate| candidate == needle)
}

fn text_value(value: &Value) -> &str {
    value["text"].as_str().unwrap()
}
