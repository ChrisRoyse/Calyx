use std::fs;
use std::path::{Path, PathBuf};

use calyx_poly::{
    LargeCorpusPage, ONCHAIN_BACKFILL_READBACK_FILE, ONCHAIN_BACKFILL_RUN_CHECKPOINT_FILE,
    ONCHAIN_BACKFILL_RUN_PASSED, ONCHAIN_BACKFILL_RUN_REPORT_FILE,
    ONCHAIN_BACKFILL_RUN_SCHEMA_VERSION, OnchainBackfillContractRun, OnchainBackfillReadbackScope,
    OnchainBackfillRunReport, RawFileState, readback_onchain_backfill_run,
    readback_onchain_backfill_run_scoped,
};
use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

#[test]
fn issue213_minimal_onchain_readback_streams_and_writes_report() {
    let (root, cleanup) =
        named_fsv_root("CALYX_POLY_ISSUE213_FSV_DIR", "issue213-onchain-readback");
    reset_dir(&root);

    let raw = root.join("raw").join("tiny_order_filled");
    fs::create_dir_all(&raw).expect("create raw dir");
    let state_path = root.join("onchain-backfill-state.json");
    let request_path = raw.join("page-000000.request.json");
    let body_path = raw.join("page-000000.json");
    let metadata_path = raw.join("page-000000.metadata.json");

    let state_bytes = br#"{"schema_version":"test.issue213.state.v1"}"#;
    let request_bytes = br#"{"jsonrpc":"2.0","method":"eth_getLogs","params":[]}"#;
    let body_bytes = br#"{"jsonrpc":"2.0","id":1,"result":[]}"#;
    fs::write(&state_path, state_bytes).expect("write state");
    fs::write(&request_path, request_bytes).expect("write request");
    fs::write(&body_path, body_bytes).expect("write body");

    let page = page(
        &request_path,
        request_bytes,
        &body_path,
        body_bytes,
        &metadata_path,
    );
    write_typed_json(&metadata_path, &page);

    let state_sha = sha256(state_bytes);
    let checkpoint_path = root.join(ONCHAIN_BACKFILL_RUN_CHECKPOINT_FILE);
    let checkpoint = checkpoint(&root, &state_path, &state_sha, &page);
    write_value_json(&checkpoint_path, &checkpoint);
    let checkpoint_sha = sha256(&fs::read(&checkpoint_path).expect("read checkpoint"));

    let run_report = OnchainBackfillRunReport {
        schema_version: ONCHAIN_BACKFILL_RUN_SCHEMA_VERSION.to_string(),
        source_of_truth: "issue213 tiny local on-chain readback corpus".to_string(),
        status_code: ONCHAIN_BACKFILL_RUN_PASSED.to_string(),
        input_state_path: state_path.display().to_string(),
        input_state_sha256: state_sha,
        output_root: root.display().to_string(),
        checkpoint_path: checkpoint_path.display().to_string(),
        checkpoint_sha256: checkpoint_sha,
        max_chunks_per_contract: 1,
        max_blocks_per_chunk: 100,
        pages: vec![page],
        contracts: vec![OnchainBackfillContractRun {
            dataset: "tiny_order_filled".to_string(),
            address: "0x0000000000000000000000000000000000000001".to_string(),
            planned_from_block: 1,
            planned_to_block: 100,
            planned_chunk_count: 1,
            start_from_block: Some(1),
            chunks_captured_this_run: 1,
            records_captured_this_run: 0,
            next_required_from_block: None,
            coverage_complete: true,
        }],
        total_pages: 1,
        total_records: 0,
        total_body_bytes: body_bytes.len() as u64,
        all_order_filled_backfill_complete: true,
        next_required_action: "read back checkpoint and proceed".to_string(),
        passed: true,
    };
    write_typed_json(&root.join(ONCHAIN_BACKFILL_RUN_REPORT_FILE), &run_report);

    let readback = readback_onchain_backfill_run(&root, 100).expect("readback passes");
    assert!(readback.passed);
    assert_eq!(
        readback.status_code,
        "POLY_ONCHAIN_BACKFILL_READBACK_PASSED"
    );
    assert_eq!(readback.total_pages, 1);
    assert_eq!(readback.current_run_page_count, 1);
    assert_eq!(readback.checkpoint_range_count, 1);
    assert_eq!(readback.checked_file_count, 6);
    assert_eq!(readback.unique_file_read_count, 6);
    assert!(readback.missing_files.is_empty());
    assert!(readback.sha_mismatches.is_empty());
    assert!(readback.parse_failures.is_empty());
    assert!(root.join(ONCHAIN_BACKFILL_READBACK_FILE).exists());

    if cleanup {
        fs::remove_dir_all(root).expect("cleanup temp FSV root");
    }
}

#[test]
fn issue213_current_run_scope_skips_prior_checkpoint_artifact_reads() {
    let (root, cleanup) = named_fsv_root(
        "CALYX_POLY_ISSUE213_SCOPE_FSV_DIR",
        "issue213-readback-scope",
    );
    reset_dir(&root);

    let raw = root.join("raw").join("tiny_order_filled");
    fs::create_dir_all(&raw).expect("create raw dir");
    let state_path = root.join("onchain-backfill-state.json");
    let state_bytes = br#"{"schema_version":"test.issue213.state.v1"}"#;
    fs::write(&state_path, state_bytes).expect("write state");

    let request_path = raw.join("page-000001.request.json");
    let body_path = raw.join("page-000001.json");
    let metadata_path = raw.join("page-000001.metadata.json");
    let request_bytes =
        br#"{"jsonrpc":"2.0","method":"eth_getLogs","params":[{"fromBlock":"0x65"}]}"#;
    let body_bytes = br#"{"jsonrpc":"2.0","id":2,"result":[]}"#;
    fs::write(&request_path, request_bytes).expect("write request");
    fs::write(&body_path, body_bytes).expect("write body");
    let current_page = page_with_range(
        &request_path,
        request_bytes,
        &body_path,
        body_bytes,
        &metadata_path,
        1,
        101,
        200,
    );
    write_typed_json(&metadata_path, &current_page);

    let state_sha = sha256(state_bytes);
    let old_request_path = raw.join("page-000000.request.json");
    let old_body_path = raw.join("page-000000.json");
    let checkpoint_path = root.join(ONCHAIN_BACKFILL_RUN_CHECKPOINT_FILE);
    let checkpoint = checkpoint_with_ranges(
        &root,
        &state_path,
        &state_sha,
        &[
            (&old_request_path, &old_body_path, 1, 100),
            (&request_path, &body_path, 101, 200),
        ],
    );
    write_value_json(&checkpoint_path, &checkpoint);
    let checkpoint_sha = sha256(&fs::read(&checkpoint_path).expect("read checkpoint"));
    let run_report = run_report(
        &root,
        &state_path,
        &state_sha,
        &checkpoint_path,
        &checkpoint_sha,
        current_page,
    );
    write_typed_json(&root.join(ONCHAIN_BACKFILL_RUN_REPORT_FILE), &run_report);

    let current =
        readback_onchain_backfill_run_scoped(&root, 100, OnchainBackfillReadbackScope::CurrentRun)
            .expect("current-run readback returns report");
    assert!(current.passed);
    assert_eq!(
        current.status_code,
        "POLY_ONCHAIN_BACKFILL_CURRENT_RUN_READBACK_PASSED"
    );
    assert_eq!(
        current.readback_scope,
        OnchainBackfillReadbackScope::CurrentRun
    );
    assert_eq!(current.current_run_page_count, 1);
    assert_eq!(current.checkpoint_range_count, 2);

    let full = readback_onchain_backfill_run(&root, 100).expect("full readback returns report");
    assert!(!full.passed);
    assert!(!full.missing_files.is_empty());

    if cleanup {
        fs::remove_dir_all(root).expect("cleanup temp FSV root");
    }
}

fn page(
    request_path: &Path,
    request_bytes: &[u8],
    body_path: &Path,
    body_bytes: &[u8],
    metadata_path: &Path,
) -> LargeCorpusPage {
    page_with_range(
        request_path,
        request_bytes,
        body_path,
        body_bytes,
        metadata_path,
        0,
        1,
        100,
    )
}

#[allow(clippy::too_many_arguments)]
fn page_with_range(
    request_path: &Path,
    request_bytes: &[u8],
    body_path: &Path,
    body_bytes: &[u8],
    metadata_path: &Path,
    page_index: usize,
    from_block: u64,
    to_block: u64,
) -> LargeCorpusPage {
    let value = json!({
        "dataset": "tiny_order_filled",
        "source": "polygon-rpc",
        "endpoint": "v2-order-filled-logs",
        "method": "POST",
        "docs_url": "local://issue213/minimal-onchain-readback",
        "page_index": page_index,
        "url": "local://polygon-rpc",
        "request_path": request_path.display().to_string(),
        "request_body_bytes": request_bytes.len() as u64,
        "request_body_sha256": sha256(request_bytes),
        "status_code": 200,
        "http_success": true,
        "expectation_met": true,
        "record_count": 0,
        "stop_reason": null,
        "body_path": body_path.display().to_string(),
        "metadata_path": metadata_path.display().to_string(),
        "body_format": "json",
        "body_bytes": body_bytes.len() as u64,
        "body_sha256": sha256(body_bytes),
        "json_parse_ok": true,
        "websocket_frame_count": null,
        "websocket_json_frame_count": null,
        "websocket_event_types": [],
        "no_payload_window": false,
        "pagination_state": null,
        "range_state": {
            "chain": "polygon",
            "address": "0x0000000000000000000000000000000000000001",
            "topics": ["0xorderfilled"],
            "from_block": from_block,
            "to_block": to_block,
            "requested_block_count": 100,
            "max_blocks_per_chunk": 100,
            "chunk_index": page_index,
            "chunk_count": 2,
            "next_from_block": if to_block == 100 { Some(101) } else { None },
            "range_policy": "fixed_100_block_rpc_chunk",
            "limit_semantics": "inclusive block range",
            "provider_limit_evidence": "issue213 synthetic local FSV corpus"
        },
        "before": raw_state(false, false, 0, None),
        "after": raw_state(true, true, body_bytes.len() as u64, Some(sha256(body_bytes)))
    });
    serde_json::from_value(value).expect("page JSON matches LargeCorpusPage")
}

fn checkpoint(root: &Path, state_path: &Path, state_sha: &str, page: &LargeCorpusPage) -> Value {
    json!({
        "schema_version": ONCHAIN_BACKFILL_RUN_SCHEMA_VERSION,
        "source_of_truth": "issue213 tiny local on-chain readback checkpoint",
        "status_code": "POLY_ONCHAIN_BACKFILL_COMPLETE",
        "input_state_path": state_path.display().to_string(),
        "input_state_sha256": state_sha,
        "chain": "polygon",
        "chain_id": 137,
        "latest_safe_block": 100,
        "max_blocks_per_chunk": 100,
        "contracts": [{
            "dataset": "tiny_order_filled",
            "address": "0x0000000000000000000000000000000000000001",
            "planned_from_block": 1,
            "planned_to_block": 100,
            "planned_chunk_count": 1,
            "captured_ranges": [{
                "from_block": 1,
                "to_block": 100,
                "record_count": 0,
                "request_path": page.request_path.as_ref().expect("request path"),
                "body_path": page.body_path.clone()
            }],
            "captured_chunk_count": 1,
            "captured_record_count": 0,
            "captured_block_count": 100,
            "next_required_from_block": null,
            "coverage_complete": true
        }],
        "all_order_filled_backfill_complete": true,
        "next_required_action": format!("read back checkpoint and proceed from {}", root.display())
    })
}

fn checkpoint_with_ranges(
    root: &Path,
    state_path: &Path,
    state_sha: &str,
    ranges: &[(&Path, &Path, u64, u64)],
) -> Value {
    let captured_ranges: Vec<Value> = ranges
        .iter()
        .map(|(request_path, body_path, from_block, to_block)| {
            json!({
                "from_block": from_block,
                "to_block": to_block,
                "record_count": 0,
                "request_path": request_path.display().to_string(),
                "body_path": body_path.display().to_string()
            })
        })
        .collect();
    json!({
        "schema_version": ONCHAIN_BACKFILL_RUN_SCHEMA_VERSION,
        "source_of_truth": "issue213 scoped on-chain readback checkpoint",
        "status_code": "POLY_ONCHAIN_BACKFILL_INCOMPLETE",
        "input_state_path": state_path.display().to_string(),
        "input_state_sha256": state_sha,
        "chain": "polygon",
        "chain_id": 137,
        "latest_safe_block": 200,
        "max_blocks_per_chunk": 100,
        "contracts": [{
            "dataset": "tiny_order_filled",
            "address": "0x0000000000000000000000000000000000000001",
            "planned_from_block": 1,
            "planned_to_block": 200,
            "planned_chunk_count": 2,
            "captured_ranges": captured_ranges,
            "captured_chunk_count": ranges.len(),
            "captured_record_count": 0,
            "captured_block_count": 100 * ranges.len() as u64,
            "next_required_from_block": null,
            "coverage_complete": true
        }],
        "all_order_filled_backfill_complete": false,
        "next_required_action": format!("read back checkpoint and proceed from {}", root.display())
    })
}

fn run_report(
    root: &Path,
    state_path: &Path,
    state_sha: &str,
    checkpoint_path: &Path,
    checkpoint_sha: &str,
    page: LargeCorpusPage,
) -> OnchainBackfillRunReport {
    OnchainBackfillRunReport {
        schema_version: ONCHAIN_BACKFILL_RUN_SCHEMA_VERSION.to_string(),
        source_of_truth: "issue213 scoped local on-chain readback corpus".to_string(),
        status_code: ONCHAIN_BACKFILL_RUN_PASSED.to_string(),
        input_state_path: state_path.display().to_string(),
        input_state_sha256: state_sha.to_string(),
        output_root: root.display().to_string(),
        checkpoint_path: checkpoint_path.display().to_string(),
        checkpoint_sha256: checkpoint_sha.to_string(),
        max_chunks_per_contract: 1,
        max_blocks_per_chunk: 100,
        pages: vec![page.clone()],
        contracts: vec![OnchainBackfillContractRun {
            dataset: "tiny_order_filled".to_string(),
            address: "0x0000000000000000000000000000000000000001".to_string(),
            planned_from_block: 1,
            planned_to_block: 200,
            planned_chunk_count: 2,
            start_from_block: Some(101),
            chunks_captured_this_run: 1,
            records_captured_this_run: 0,
            next_required_from_block: None,
            coverage_complete: true,
        }],
        total_pages: 1,
        total_records: 0,
        total_body_bytes: page.body_bytes,
        all_order_filled_backfill_complete: false,
        next_required_action: "scoped current-run readback only".to_string(),
        passed: true,
    }
}

fn raw_state(
    body_exists: bool,
    metadata_exists: bool,
    body_bytes: u64,
    body_sha256: Option<String>,
) -> RawFileState {
    RawFileState {
        body_exists,
        metadata_exists,
        body_bytes,
        body_sha256,
    }
}

fn write_typed_json<T: Serialize>(path: &Path, value: &T) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create parent");
    }
    fs::write(
        path,
        serde_json::to_vec_pretty(value).expect("encode typed JSON"),
    )
    .expect("write typed JSON");
}

fn write_value_json(path: &Path, value: &Value) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create parent");
    }
    fs::write(
        path,
        serde_json::to_vec_pretty(value).expect("encode value JSON"),
    )
    .expect("write value JSON");
}

fn sha256(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

fn named_fsv_root(env: &str, fallback_name: &str) -> (PathBuf, bool) {
    if let Some(value) = std::env::var_os(env) {
        return (PathBuf::from(value), false);
    }
    (
        std::env::temp_dir().join(format!("{fallback_name}-{}", std::process::id())),
        true,
    )
}

fn reset_dir(path: &Path) {
    if path.exists() {
        fs::remove_dir_all(path).expect("remove previous FSV root");
    }
    fs::create_dir_all(path).expect("create FSV root");
}
