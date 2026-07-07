use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use calyx_poly::{
    SCHEMA_DERIVATION_PASSED, SchemaContract, SchemaDerivationRequest,
    read_schema_derivation_report, require_schema_derivation_passed, run_schema_derivation,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

#[test]
fn issue174_derives_schema_from_physical_corpus_and_fails_closed_on_edges() {
    let corpus = write_known_truth_corpus();

    let ok_output = corpus.join("schema-ok");
    let mut ok_request = SchemaDerivationRequest::new(&corpus, &ok_output);
    ok_request.required_sources = vec!["clob".to_string(), "gamma".to_string()];
    ok_request.required_join_keys =
        vec!["condition_id".to_string(), "token_or_asset_id".to_string()];
    let ok_report = run_schema_derivation(&ok_request).expect("schema derivation should run");
    require_schema_derivation_passed(&ok_report).expect("known-truth schema derivation passes");
    assert_eq!(ok_report.status_code, SCHEMA_DERIVATION_PASSED);
    assert!(ok_report.after_files.iter().all(|file| file.exists));
    assert!(ok_report.nullable_or_union_field_count >= 1);
    assert!(
        ok_report
            .blocked_runtime_sources
            .iter()
            .any(|source| source.issue == "#198")
    );
    let persisted_ok_report =
        read_schema_derivation_report(&ok_output.join("schema-derivation-report.json"))
            .expect("persisted schema report should parse");
    assert_eq!(persisted_ok_report.status_code, SCHEMA_DERIVATION_PASSED);
    let contract: SchemaContract =
        read_json(&ok_output.join("schema-contract.json")).expect("contract should parse");
    assert!(
        contract.field_contracts.iter().any(|field| {
            field.field == "feeType" && field.variant_contract == "union_required"
        })
    );

    let missing_source_output = corpus.join("schema-missing-source");
    let mut missing_source_request = SchemaDerivationRequest::new(&corpus, &missing_source_output);
    missing_source_request.required_sources = vec![
        "clob".to_string(),
        "gamma".to_string(),
        "websocket-rtds".to_string(),
    ];
    missing_source_request.required_join_keys =
        vec!["condition_id".to_string(), "token_or_asset_id".to_string()];
    let missing_source_report =
        run_schema_derivation(&missing_source_request).expect("missing-source report is written");
    assert!(!missing_source_report.passed);
    assert_eq!(
        missing_source_report.status_code,
        "POLY_SCHEMA_DERIVATION_SOURCE_MISSING"
    );
    assert!(require_schema_derivation_passed(&missing_source_report).is_err());
    assert!(
        missing_source_output
            .join("schema-derivation-report.json")
            .exists()
    );

    let missing_join_output = corpus.join("schema-missing-join");
    let mut missing_join_request = SchemaDerivationRequest::new(&corpus, &missing_join_output);
    missing_join_request.required_sources = vec!["clob".to_string(), "gamma".to_string()];
    missing_join_request.required_join_keys = vec![
        "condition_id".to_string(),
        "token_or_asset_id".to_string(),
        "transaction_hash".to_string(),
    ];
    let missing_join_report =
        run_schema_derivation(&missing_join_request).expect("missing-join report is written");
    assert!(!missing_join_report.passed);
    assert_eq!(
        missing_join_report.status_code,
        "POLY_SCHEMA_DERIVATION_JOIN_KEY_MISSING"
    );
    assert!(require_schema_derivation_passed(&missing_join_report).is_err());
    assert!(
        missing_join_output
            .join("schema-derivation-report.json")
            .exists()
    );
}

fn write_known_truth_corpus() -> PathBuf {
    let root = unique_root();
    fs::create_dir_all(root.join("raw/gamma_markets")).expect("create gamma raw dir");
    fs::create_dir_all(root.join("raw/clob_books")).expect("create clob raw dir");
    fs::create_dir_all(root.join("raw/ws_equity")).expect("create websocket raw dir");
    fs::create_dir_all(root.join("profiles")).expect("create profiles dir");
    fs::create_dir_all(root.join("schema-observations")).expect("create observation dir");

    let gamma_body = root.join("raw/gamma_markets/page-000000.json");
    let (gamma_sha, gamma_len) = write_bytes(
        &gamma_body,
        br#"[{"conditionId":"0xcond1","questionID":"q1","feeType":null},{"conditionId":"0xcond2","questionID":"q2","feeType":"fixed"}]"#,
    );
    let gamma_metadata = root.join("raw/gamma_markets/page-000000.metadata.json");
    let (gamma_meta_sha, _) = write_json_file(&gamma_metadata, json!({"source":"gamma"}));

    let clob_body = root.join("raw/clob_books/page-000000.json");
    let (clob_sha, clob_len) = write_json_file(
        &clob_body,
        json!({"asset_id":"token-1","condition_id":"0xcond1","bids":[{"price":"0.41"}]}),
    );
    let clob_metadata = root.join("raw/clob_books/page-000000.metadata.json");
    let (clob_meta_sha, _) = write_json_file(&clob_metadata, json!({"source":"clob"}));

    let equity_body = root.join("raw/ws_equity/edge-000000.json");
    let (equity_sha, _) = write_json_file(&equity_body, json!([]));
    let equity_metadata = root.join("raw/ws_equity/edge-000000.metadata.json");
    let (equity_meta_sha, _) = write_json_file(&equity_metadata, json!({"source":"rtds"}));

    let gamma_profile = root.join("profiles/gamma_markets.json");
    write_json_file(
        &gamma_profile,
        json!({
            "dataset":"gamma_markets",
            "source":"gamma",
            "record_count":2,
            "fields":[
                field("conditionId", 2, 0, 0, json!({"string":2})),
                field("questionID", 2, 0, 0, json!({"string":2})),
                field("feeType", 2, 0, 1, json!({"null":1,"string":1}))
            ]
        }),
    );
    let clob_profile = root.join("profiles/clob_books.json");
    write_json_file(
        &clob_profile,
        json!({
            "dataset":"clob_books",
            "source":"clob",
            "record_count":1,
            "fields":[
                field("asset_id", 1, 0, 0, json!({"string":1})),
                field("condition_id", 1, 0, 0, json!({"string":1})),
                field("price", 1, 0, 0, json!({"string":1}))
            ]
        }),
    );
    let join_profile = root.join("join-profile.json");
    write_json_file(
        &join_profile,
        json!({
            "schema_version":"poly.large_corpus.join_profile.v1",
            "record_count":3,
            "identifier_counts":{"condition_id":3,"token_or_asset_id":1},
            "examples":{"condition_id":["0xcond1"],"token_or_asset_id":["token-1"]}
        }),
    );
    let decision_input = root.join("schema-decision-input.md");
    fs::write(
        &decision_input,
        "known-truth corpus schema decision input\n",
    )
    .expect("write schema decision input");

    write_ws_semantics(
        &root,
        &gamma_body,
        &gamma_sha,
        &gamma_metadata,
        &gamma_meta_sha,
    );
    write_manifest(
        &root,
        &gamma_body,
        &gamma_metadata,
        &gamma_sha,
        gamma_len,
        &clob_body,
        &clob_metadata,
        &clob_sha,
        clob_len,
        &equity_body,
        &equity_metadata,
        &equity_sha,
        &gamma_profile,
        &clob_profile,
        &join_profile,
        &decision_input,
    );
    assert_eq!(gamma_meta_sha.len(), 64);
    assert_eq!(clob_meta_sha.len(), 64);
    assert_eq!(equity_meta_sha.len(), 64);
    root
}

fn field(name: &str, present: usize, missing: usize, nulls: usize, types: Value) -> Value {
    json!({
        "name":name,
        "present_count":present,
        "missing_count":missing,
        "null_count":nulls,
        "type_counts":types,
        "json_string_count":0,
        "array_min_len":null,
        "array_max_len":null,
        "example_sha256":null
    })
}

#[allow(clippy::too_many_arguments)]
fn write_manifest(
    root: &Path,
    gamma_body: &Path,
    gamma_metadata: &Path,
    gamma_sha: &str,
    gamma_len: u64,
    clob_body: &Path,
    clob_metadata: &Path,
    clob_sha: &str,
    clob_len: u64,
    equity_body: &Path,
    equity_metadata: &Path,
    equity_sha: &str,
    gamma_profile: &Path,
    clob_profile: &Path,
    join_profile: &Path,
    decision_input: &Path,
) {
    write_json_file(
        &root.join("large-corpus-manifest.json"),
        json!({
            "schema_version":"poly.large_corpus.v1",
            "captured_at_unix_ms":now_ms(),
            "source_of_truth":"physical known-truth Polymarket-shaped corpus files",
            "page_size":2,
            "max_pages_per_dataset":1,
            "pages":[
                page("gamma_markets", "gamma", "markets", gamma_body, gamma_metadata, gamma_sha, gamma_len, 2),
                page("clob_books", "clob", "book", clob_body, clob_metadata, clob_sha, clob_len, 1)
            ],
            "edge_cases":[edge_case(equity_body, equity_metadata, equity_sha)],
            "field_profile_paths":[path_string(gamma_profile), path_string(clob_profile)],
            "join_profile_path":path_string(join_profile),
            "schema_decision_input_path":path_string(decision_input),
            "total_pages":2,
            "total_records":3,
            "total_body_bytes":gamma_len + clob_len,
            "passed":true,
            "status_code":"POLY_LARGE_CORPUS_CAPTURE_PASSED",
            "failure":null
        }),
    );
}

#[allow(clippy::too_many_arguments)]
fn page(
    dataset: &str,
    source: &str,
    endpoint: &str,
    body_path: &Path,
    metadata_path: &Path,
    body_sha: &str,
    body_bytes: u64,
    record_count: usize,
) -> Value {
    json!({
        "dataset":dataset,
        "source":source,
        "endpoint":endpoint,
        "method":"GET",
        "docs_url":"https://docs.polymarket.com/",
        "page_index":0,
        "url":"https://fixture.invalid",
        "request_path":null,
        "request_body_bytes":0,
        "request_body_sha256":null,
        "status_code":200,
        "http_success":true,
        "expectation_met":true,
        "record_count":record_count,
        "stop_reason":null,
        "body_path":path_string(body_path),
        "metadata_path":path_string(metadata_path),
        "body_format":"json",
        "body_bytes":body_bytes,
        "body_sha256":body_sha,
        "json_parse_ok":true,
        "websocket_frame_count":null,
        "websocket_json_frame_count":null,
        "websocket_event_types":[],
        "no_payload_window":false,
        "range_state":null,
        "before":raw_state(false, 0, Value::Null),
        "after":raw_state(true, body_bytes, json!(body_sha))
    })
}

fn edge_case(body_path: &Path, metadata_path: &Path, body_sha: &str) -> Value {
    json!({
        "name":"ws_rtds_equity_known_no_payload",
        "method":"WS",
        "url":"wss://ws-live-data.polymarket.com",
        "request_path":null,
        "request_body_bytes":0,
        "request_body_sha256":null,
        "expected_semantics":"documented_but_no_payload_window",
        "status_code":101,
        "expectation_met":true,
        "record_count":0,
        "body_path":path_string(body_path),
        "metadata_path":path_string(metadata_path),
        "body_format":"json",
        "json_parse_ok":true,
        "body_sha256":body_sha,
        "websocket_frame_count":Some(1),
        "websocket_json_frame_count":Some(0),
        "websocket_event_types":[],
        "no_payload_window":true,
        "range_state":null,
        "before":raw_state(false, 0, Value::Null),
        "after":raw_state(true, 2, json!(body_sha))
    })
}

fn write_ws_semantics(
    root: &Path,
    body_path: &Path,
    body_sha: &str,
    metadata_path: &Path,
    metadata_sha: &str,
) {
    let observations = (0..3)
        .map(|index| {
            json!({
                "sample_name":format!("ws_fixture_{index}"),
                "source":"websocket-market",
                "endpoint":"market",
                "method":"WS",
                "docs_url":"https://docs.polymarket.com/",
                "request_case":"control",
                "expected_runtime_semantics":"payload",
                "actual_status_code":101,
                "actual_body_shape":"json",
                "actual_body_fields":["conditionId"],
                "websocket_frame_count":Some(1),
                "websocket_json_frame_count":Some(1),
                "websocket_event_types":["book"],
                "no_payload_window":false,
                "semantics_match":true,
                "failure_code":null,
                "schema_implication":"windowed-frame-stream",
                "request_body_path":null,
                "request_body_sha256":null,
                "body_path":path_string(body_path),
                "metadata_path":path_string(metadata_path),
                "body_bytes":1,
                "body_sha256":body_sha,
                "metadata_sha256":metadata_sha,
                "before":raw_state(false, 0, Value::Null),
                "after":raw_state(true, 1, json!(body_sha))
            })
        })
        .collect::<Vec<_>>();
    write_json_file(
        &root.join("schema-observations/websocket-runtime-semantics.json"),
        json!({
            "schema_version":"poly.large_corpus.websocket_runtime_semantics.v1",
            "source_of_truth":"physical fixture files",
            "observation_count":observations.len(),
            "observations":observations,
            "passed":true,
            "failure":null
        }),
    );
}

fn raw_state(exists: bool, body_bytes: u64, sha: Value) -> Value {
    json!({
        "body_exists":exists,
        "metadata_exists":exists,
        "body_bytes":body_bytes,
        "body_sha256":sha
    })
}

fn write_json_file(path: &Path, value: Value) -> (String, u64) {
    let bytes = serde_json::to_vec_pretty(&value).expect("serialize JSON");
    write_bytes(path, &bytes)
}

fn write_bytes(path: &Path, bytes: &[u8]) -> (String, u64) {
    fs::write(path, bytes).unwrap_or_else(|err| panic!("write {}: {err}", path.display()));
    (sha256_hex(bytes), bytes.len() as u64)
}

fn read_json<T: for<'de> serde::Deserialize<'de>>(path: &Path) -> serde_json::Result<T> {
    let bytes = fs::read(path).expect("read JSON file");
    serde_json::from_slice(&bytes)
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn unique_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .join("target/fsv")
        .join(format!(
            "issue174_schema_derivation_test_{}_{}",
            std::process::id(),
            now_ms()
        ))
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock after Unix epoch")
        .as_millis()
}

fn path_string(path: &Path) -> String {
    path.display().to_string()
}
