use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use calyx_aster::cf::{ColumnFamily, base_key};
use calyx_aster::vault::{AsterVault, VaultOptions};
use calyx_core::{
    Constellation, CxFlags, InputRef, LedgerRef, Modality, SlotId, SlotVector, VaultId, VaultStore,
};
use calyx_sextant::{RerankCandidateText, RerankRequest, RerankerClient};
use serde_json::json;

mod reranker_support;
#[path = "sextant_support/mod.rs"]
mod sextant_support;
use reranker_support::spawn_reranker;
use sextant_support::{cx_u8_fill as cx, hex, raw_blake3_hex as blake3_hex};

#[test]
fn issue594_reranker_candidate_text_non_persistence_manual_fsv() {
    let root = fsv_root().join("issue594-reranker-candidate-nonpersistence");
    reset_dir(&root);
    let vault_dir = root.join("vault");
    let vault = open_vault(&vault_dir);
    let stored = persisted_constellation();
    let stored_key = base_key(stored.cx_id);
    vault.put(stored).expect("persist safe constellation");
    vault.flush().expect("flush safe constellation");
    let base_row = vault
        .read_cf_at(vault.snapshot(), ColumnFamily::Base, &stored_key)
        .expect("read base row before rerank");

    let happy = "ISSUE594_CANDIDATE_SENTINEL_happy_do_not_persist";
    let long = format!("ISSUE594_CANDIDATE_SENTINEL_long_{}", "x".repeat(8192));
    let fail = "ISSUE594_CANDIDATE_SENTINEL_fail_closed_do_not_persist";
    let before_happy_hits = scan_dir_for_bytes(&vault_dir, happy.as_bytes());
    let happy_observation = run_successful_rerank(happy);
    let after_happy_hits = scan_dir_for_bytes(&vault_dir, happy.as_bytes());
    let empty_observation = run_successful_rerank("");
    let before_long_hits = scan_dir_for_bytes(&vault_dir, long.as_bytes());
    let long_observation = run_successful_rerank(&long);
    let after_long_hits = scan_dir_for_bytes(&vault_dir, long.as_bytes());
    let fail_request = RerankRequest::new("privacy query", vec![fail.to_string()]);
    let fail_error = RerankerClient::new("not-http://127.0.0.1", Duration::from_millis(1))
        .rerank(&fail_request)
        .expect_err("invalid endpoint must fail closed");
    let after_fail_hits = scan_dir_for_bytes(&vault_dir, fail.as_bytes());
    let vault_files = list_files(&vault_dir);

    let readback = json!({
        "issue": 594,
        "trigger": "RerankerClient::rerank with request-scoped candidate text",
        "expected": "candidate text reaches the reranker request but is absent from persisted Aster vault bytes",
        "source_of_truth": {
            "vault_dir": vault_dir,
            "persisted_surfaces": ["cf/base", "cf/ledger", "wal", "current manifest"],
            "base_row_present_after_safe_persist": base_row.is_some(),
            "base_key_hex": hex(&stored_key),
            "vault_file_count": vault_files.len(),
            "vault_files": vault_files,
        },
        "happy_path": {
            "candidate_hash": blake3_hex(happy.as_bytes()),
            "before_hits": before_happy_hits,
            "request_contained_candidate": happy_observation.request_contained_candidate,
            "score": happy_observation.score,
            "after_hits": after_happy_hits,
        },
        "edges": {
            "empty_candidate": {
                "candidate_len": 0,
                "request_text_count": empty_observation.request_text_count,
                "score": empty_observation.score,
            },
            "long_candidate": {
                "candidate_len": long.len(),
                "candidate_hash": blake3_hex(long.as_bytes()),
                "before_hits": before_long_hits,
                "request_contained_candidate": long_observation.request_contained_candidate,
                "after_hits": after_long_hits,
            },
            "invalid_endpoint": {
                "candidate_hash": blake3_hex(fail.as_bytes()),
                "error_code": fail_error.code,
                "after_hits": after_fail_hits,
            }
        },
        "request_type": {
            "candidate_item_type": std::any::type_name::<RerankCandidateText>(),
            "request_debug_redacted": !format!("{:?}", RerankRequest::new("privacy query", vec![happy.to_string()])).contains(happy),
        }
    });
    let report_path = root.join("issue594-reranker-nonpersistence-readback.json");
    fs::write(&report_path, serde_json::to_vec_pretty(&readback).unwrap()).unwrap();

    println!("ISSUE594_FSV_ROOT={}", root.display());
    println!("ISSUE594_FSV_REPORT={}", report_path.display());
    println!("{}", serde_json::to_string_pretty(&readback).unwrap());

    assert!(base_row.is_some());
    assert!(
        readback["happy_path"]["before_hits"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(
        readback["happy_path"]["after_hits"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(happy_observation.score, 0.42);
    assert_eq!(empty_observation.request_text_count, 1);
    assert!(
        readback["edges"]["long_candidate"]["before_hits"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(
        readback["edges"]["long_candidate"]["after_hits"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(fail_error.code, "CALYX_SEXTANT_RERANKER_ENDPOINT");
    assert!(
        readback["edges"]["invalid_endpoint"]["after_hits"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(readback["request_type"]["request_debug_redacted"], true);
}

struct RerankObservation {
    request_contained_candidate: bool,
    request_text_count: usize,
    score: f32,
}

fn run_successful_rerank(candidate: &str) -> RerankObservation {
    let server = spawn_reranker("HTTP/1.1 200 OK", r#"{"scores":[0.42]}"#);
    let response = RerankerClient::new(server.endpoint.clone(), Duration::from_secs(1))
        .rerank(&RerankRequest::new(
            "privacy query",
            vec![candidate.to_string()],
        ))
        .expect("rerank request completes");
    server.join();
    let request = server.request();
    let texts = request_texts(request_body(&request));
    RerankObservation {
        request_contained_candidate: texts.first().is_some_and(|text| text == candidate),
        request_text_count: texts.len(),
        score: response.scores[0],
    }
}

fn request_body(request: &str) -> &str {
    request.split("\r\n\r\n").nth(1).unwrap()
}

fn request_texts(body: &str) -> Vec<String> {
    serde_json::from_str::<serde_json::Value>(body).unwrap()["texts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|value| value.as_str().unwrap().to_string())
        .collect()
}

fn open_vault(path: &Path) -> AsterVault {
    AsterVault::new_durable(
        path,
        vault_id(),
        b"issue594-salt".to_vec(),
        VaultOptions::default(),
    )
    .expect("open durable vault")
}

fn persisted_constellation() -> Constellation {
    let mut slots = BTreeMap::new();
    slots.insert(
        SlotId::new(0),
        SlotVector::Dense {
            dim: 2,
            data: vec![0.25, 0.75],
        },
    );
    Constellation {
        cx_id: cx(90),
        vault_id: vault_id(),
        panel_version: 61,
        created_at: 1_786_600_594,
        input_ref: InputRef {
            hash: [0x59; 32],
            pointer: Some("zfs://calyx/issue594/safe-source".to_string()),
            redacted: false,
        },
        modality: Modality::Text,
        slots,
        scalars: BTreeMap::new(),
        metadata: BTreeMap::new(),
        anchors: Vec::new(),
        provenance: LedgerRef {
            seq: 0,
            hash: [0; 32],
        },
        flags: CxFlags::default(),
    }
}

fn vault_id() -> VaultId {
    "01ARZ3NDEKTSV4RRFFQ69G5FAV"
        .parse()
        .expect("valid vault id")
}

fn fsv_root() -> PathBuf {
    std::env::var_os("CALYX_FSV_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("calyx-issue594-fsv"))
}

fn reset_dir(path: &Path) {
    if path.exists() {
        fs::remove_dir_all(path).expect("remove stale fsv dir");
    }
    fs::create_dir_all(path).expect("create fsv dir");
}

fn scan_dir_for_bytes(root: &Path, needle: &[u8]) -> Vec<String> {
    let mut hits = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).expect("read scan dir") {
            let path = entry.expect("read scan entry").path();
            if path.is_dir() {
                stack.push(path);
            } else if fs::read(&path)
                .expect("read scan file")
                .windows(needle.len())
                .any(|window| window == needle)
            {
                hits.push(relative_path(root, &path));
            }
        }
    }
    hits.sort();
    hits
}

fn list_files(root: &Path) -> Vec<String> {
    let mut files = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).expect("read vault dir") {
            let path = entry.expect("read vault entry").path();
            if path.is_dir() {
                stack.push(path);
            } else {
                files.push(relative_path(root, &path));
            }
        }
    }
    files.sort();
    files
}

fn relative_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .display()
        .to_string()
}
