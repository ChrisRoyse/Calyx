use std::fs;
use std::path::{Path, PathBuf};

use serde_json::json;

use super::*;

#[test]
fn corpus_readback_reports_manifest_and_files() {
    let root = temp_root("corpus-readback-ok");
    fs::create_dir_all(root.join("image")).unwrap();
    fs::create_dir_all(root.join("edges")).unwrap();
    write(&root.join("image/rows.jsonl"), b"{\"id\":1}\n{\"id\":2}\n");
    for edge in ["empty.bin", "wrong_modality.bin", "missing_label.bin"] {
        write(&root.join("edges").join(edge), edge.as_bytes());
    }
    write_manifest(&root, "image/rows.jsonl", "cifar10", "image");

    let got = readback(&root).unwrap();
    assert_eq!(got.schema, SCHEMA);
    assert_eq!(got.lanes.len(), 1);
    assert_eq!(got.lanes[0].files[0].rows, Some(2));
    assert_eq!(got.edge_cases.len(), 3);
    assert_eq!(got.total_samples, 2);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn corpus_readback_fails_closed_on_hash_mismatch() {
    let root = temp_root("corpus-readback-sha");
    fs::create_dir_all(root.join("image")).unwrap();
    fs::create_dir_all(root.join("edges")).unwrap();
    write(&root.join("image/rows.jsonl"), b"{\"id\":1}\n");
    for edge in ["empty.bin", "wrong_modality.bin", "missing_label.bin"] {
        write(&root.join("edges").join(edge), edge.as_bytes());
    }
    write_manifest(&root, "image/rows.jsonl", "cifar10", "image");
    fs::write(root.join("image/rows.jsonl"), b"tampered\n").unwrap();

    let err = readback(&root).unwrap_err();
    assert!(err.contains("CALYX_FSV_CORPUS_SHA_MISMATCH"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn corpus_readback_fails_closed_on_path_escape() {
    let root = temp_root("corpus-readback-escape");
    fs::create_dir_all(&root).unwrap();
    let manifest = json!({
        "schema": SCHEMA,
        "bundle_id": "bad",
        "sources": [{"id":"s","url":"https://example.invalid","license":"test","revision":"1"}],
        "lanes": [{
            "name":"image",
            "modality":"image",
            "source":"s",
            "sample_count":1,
            "files":[{"path":"../secret","role":"rows","sha256":"0".repeat(64),"bytes":0}]
        }],
        "edge_cases": []
    });
    fs::write(
        root.join("manifest.json"),
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();

    let err = readback(&root).unwrap_err();
    assert!(err.contains("CALYX_FSV_CORPUS_PATH_ESCAPE"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn corpus_readback_requires_three_edge_cases() {
    let root = temp_root("corpus-readback-edges");
    fs::create_dir_all(root.join("image")).unwrap();
    write(&root.join("image/rows.jsonl"), b"{\"id\":1}\n");
    let file = file_json("image/rows.jsonl", &root.join("image/rows.jsonl"), Some(1));
    let manifest = json!({
        "schema": SCHEMA,
        "bundle_id": "few-edges",
        "sources": [{"id":"s","url":"https://example.invalid","license":"test","revision":"1"}],
        "lanes": [{
            "name":"image",
            "modality":"image",
            "source":"s",
            "sample_count":1,
            "files":[file]
        }],
        "edge_cases": []
    });
    fs::write(
        root.join("manifest.json"),
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();

    let err = readback(&root).unwrap_err();
    assert!(err.contains("CALYX_FSV_CORPUS_EDGE_CASES_MISSING"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn corpus_readback_fails_closed_on_unknown_manifest_fields() {
    let root = temp_root("corpus-readback-unknown");
    fs::create_dir_all(&root).unwrap();
    let manifest = json!({
        "schema": SCHEMA,
        "bundle_id": "bad",
        "unexpected": "field",
        "sources": [{"id":"s","url":"https://example.invalid","license":"test","revision":"1"}],
        "lanes": [],
        "edge_cases": []
    });
    fs::write(
        root.join("manifest.json"),
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();

    let err = readback(&root).unwrap_err();
    assert!(err.contains("CALYX_FSV_CORPUS_INVALID_MANIFEST"));
    let _ = fs::remove_dir_all(root);
}

fn write_manifest(root: &Path, rows: &str, source: &str, modality: &str) {
    let row_path = root.join(rows);
    let edge_names = ["empty.bin", "wrong_modality.bin", "missing_label.bin"];
    let edges: Vec<_> = edge_names
        .iter()
        .map(|name| {
            let rel = format!("edges/{name}");
            let path = root.join(&rel);
            let mut file = file_json(&rel, &path, None);
            file.as_object_mut().unwrap().remove("role");
            file["expected_error"] = json!("CALYX_FSV_EDGE");
            file["lane"] = json!(modality);
            file["name"] = json!(name.trim_end_matches(".bin"));
            file
        })
        .collect();
    let manifest = json!({
        "schema": SCHEMA,
        "bundle_id": "test-bundle",
        "sources": [{
            "id": source,
            "url": "https://example.invalid/dataset",
            "license": "test-license",
            "revision": "test-revision"
        }],
        "lanes": [{
            "name": modality,
            "modality": modality,
            "source": source,
            "sample_count": 2,
            "expected_labels": ["a", "b"],
            "files": [file_json(rows, &row_path, Some(2))]
        }],
        "edge_cases": edges
    });
    fs::write(
        root.join("manifest.json"),
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();
}

fn file_json(rel: &str, path: &Path, rows: Option<usize>) -> serde_json::Value {
    let bytes = fs::read(path).unwrap();
    let mut value = json!({
        "path": rel,
        "role": "rows",
        "sha256": sha256_hex(&bytes),
        "bytes": bytes.len(),
    });
    if let Some(rows) = rows {
        value["rows"] = json!(rows);
    }
    value
}

fn write(path: &Path, bytes: &[u8]) {
    fs::write(path, bytes).unwrap();
}

fn temp_root(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("{name}-{}", std::process::id()))
}
