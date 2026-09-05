//! Issue #173 - raw-source physical readback verifier.
//!
//! Source of truth: files on disk named by source-inventory.json.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use calyx_poly::{
    RAW_SOURCE_INVENTORY_SCHEMA_VERSION, RAW_SOURCE_READBACK_PASSED, RawDocsIndexCoverage,
    RawDocsIndexSnapshot, RawEndpointSample, RawFileState, RawJoinMap, RawSourceCoverage,
    RawSourceInventory, readback_raw_source_inventory, require_raw_source_readback_passed,
};
use sha2::{Digest, Sha256};

#[allow(dead_code)]
// calyx-shared-module: path=fsv_support.rs alias=__calyx_shared_fsv_support_rs local=support visibility=private
use crate::__calyx_shared_fsv_support_rs as support;

use support::{named_fsv_root, reset_dir};

#[test]
fn issue173_raw_source_readback_detects_physical_mismatch() {
    let (root, _keep) = named_fsv_root(
        "POLY_ISSUE173_READBACK_FSV_ROOT",
        "issue173-raw-source-readback",
    );
    reset_dir(&root);
    let body = br#"[{"conditionId":"0x173","clobTokenIds":"[\"tokA\",\"tokB\"]"}]"#;
    let request = br#"{"limit":1}"#;
    let sample = write_inventory(&root, body, request);

    let pass_report = readback_raw_source_inventory(&root).expect("readback pass");
    require_raw_source_readback_passed(&pass_report).expect("physical readback passes");
    assert_eq!(pass_report.status_code, RAW_SOURCE_READBACK_PASSED);
    assert_eq!(pass_report.sample_count, 1);
    assert_eq!(pass_report.failure_count, 0);

    fs::write(&sample.body_path, br#"{"corrupt":true}"#).expect("corrupt body");
    let fail_report = readback_raw_source_inventory(&root).expect("readback fail report");
    assert!(require_raw_source_readback_passed(&fail_report).is_err());
    assert_eq!(fail_report.failure_count, 1);
    assert!(
        fail_report
            .files
            .iter()
            .any(|file| file.sample_name == "issue173_live_shape"
                && file.role == "body"
                && !file.passed)
    );
}

fn write_inventory(root: &Path, body: &[u8], request: &[u8]) -> RawEndpointSample {
    let sample_dir = root.join("raw").join("gamma").join("issue173_live_shape");
    fs::create_dir_all(&sample_dir).expect("sample dir");
    let body_path = sample_dir.join("body.json");
    let request_path = sample_dir.join("request.json");
    let metadata_path = sample_dir.join("metadata.json");
    fs::write(&body_path, body).expect("body");
    fs::write(&request_path, request).expect("request");
    let sample = RawEndpointSample {
        name: "issue173_live_shape".to_string(),
        source: "gamma".to_string(),
        transport: "http".to_string(),
        endpoint: "markets".to_string(),
        method: "GET".to_string(),
        url: "https://gamma-api.polymarket.com/markets?limit=1".to_string(),
        docs_url: "https://docs.polymarket.com/api-reference/markets/list-markets".to_string(),
        request_body_exists: true,
        request_body_bytes: request.len() as u64,
        request_body_sha256: Some(sha256_hex(request)),
        request_body_path: Some(path_string(&request_path)),
        expected_success: true,
        edge_case: false,
        status_code: Some(200),
        http_success: true,
        expectation_met: true,
        error_code: None,
        error_message: None,
        body_exists: true,
        body_bytes: body.len() as u64,
        body_sha256: Some(sha256_hex(body)),
        json_parse_ok: true,
        record_count: Some(1),
        top_level_fields: vec!["clobTokenIds".to_string(), "conditionId".to_string()],
        websocket_frame_count: None,
        websocket_json_frame_count: None,
        websocket_event_types: Vec::new(),
        websocket_pong_received: None,
        websocket_outbound_messages: Vec::new(),
        websocket_frames: Vec::new(),
        before: empty_state(),
        after: RawFileState {
            body_exists: true,
            metadata_exists: false,
            body_bytes: body.len() as u64,
            body_sha256: Some(sha256_hex(body)),
        },
        body_path: path_string(&body_path),
        metadata_path: path_string(&metadata_path),
    };
    write_json(&metadata_path, &sample);
    let inventory = inventory(root, sample.clone());
    write_json(&root.join("source-inventory.json"), &inventory);
    sample
}

fn inventory(root: &Path, sample: RawEndpointSample) -> RawSourceInventory {
    RawSourceInventory {
        schema_version: RAW_SOURCE_INVENTORY_SCHEMA_VERSION.to_string(),
        captured_at_unix_ms: 1783338000000,
        source_of_truth: "known physical issue173 readback fixture".to_string(),
        docs: vec![sample.docs_url.clone()],
        samples: vec![sample],
        join_map: RawJoinMap::default(),
        coverage: RawSourceCoverage {
            sample_count: 1,
            required_success_count: 1,
            required_failure_count: 0,
            edge_case_count: 0,
            total_body_bytes: 1,
            readback_sha_mismatches: 0,
            sampled_sources: vec!["gamma".to_string()],
            unsampled_sources: Vec::new(),
        },
        docs_index_coverage: docs_coverage(root),
        schema_observations: Vec::new(),
        runtime_semantics: Vec::new(),
        passed: true,
        status_code: "POLY_RAW_SOURCE_SAMPLE_PASSED".to_string(),
        failure: None,
    }
}

fn docs_coverage(root: &Path) -> RawDocsIndexCoverage {
    RawDocsIndexCoverage {
        schema_version: "poly.docs_index_coverage.v1".to_string(),
        captured_at_unix_ms: 1783338000000,
        docs_index: RawDocsIndexSnapshot {
            url: "https://docs.polymarket.com/llms.txt".to_string(),
            status_code: 200,
            body_path: path_string(&root.join("docs.txt")),
            metadata_path: path_string(&root.join("docs.metadata.json")),
            body_bytes: 0,
            body_sha256: sha256_hex(b""),
            before: empty_state(),
            after: empty_state(),
        },
        row_count: 0,
        classification_counts: BTreeMap::new(),
        rows: Vec::new(),
        artifact_failures: Vec::new(),
        not_yet_sampled_count: 0,
        blocked_runtime_count: 0,
        forbidden_count: 0,
        passed: true,
        status_code: "POLY_RAW_SOURCE_DOCS_COVERAGE_PASSED".to_string(),
        failure: None,
    }
}

fn write_json<T: serde::Serialize>(path: &Path, value: &T) {
    let bytes = serde_json::to_vec_pretty(value).expect("json");
    fs::write(path, bytes).expect("write json");
}

fn path_string(path: &Path) -> String {
    path.display().to_string()
}

fn empty_state() -> RawFileState {
    RawFileState {
        body_exists: false,
        metadata_exists: false,
        body_bytes: 0,
        body_sha256: None,
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}
