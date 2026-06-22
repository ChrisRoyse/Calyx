//! Issue #594 FSV against the REAL resident TEI reranker (:8089): candidate
//! text is request-scoped and never persisted (PRD 30 §1/§4, 10 §7).
//!
//! Complements `reranker_nonpersistence_fsv.rs` (mock server + Aster vault
//! byte-scan): this test drives the real cross-encoder, and the operator
//! follows it with an independent byte-scan of ALL persisted state on
//! manual (calyx data/logs/tmp, /tmp, TEI docker logs, journald).
//!
//! The sentinel is injected via `CALYX_FSV_SENTINEL` so it exists neither in
//! this source file nor in the compiled binary — the scan therefore has no
//! false positives. Evidence records only the sentinel's blake3 hash.

use std::fs;
use std::time::Duration;

use calyx_sextant::{RerankRequest, RerankerClient};
use serde_json::json;

fn required_env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| {
        panic!("{name} must be set; this FSV refuses to run with defaults baked in")
    })
}

#[test]
#[ignore = "manual FSV: requires resident TEI :8089 and an on-disk sentinel byte-scan"]
fn reranker_candidate_text_never_persists_real_tei() {
    let sentinel = required_env("CALYX_FSV_SENTINEL");
    let root = std::path::PathBuf::from(required_env("CALYX_FSV_ROOT"));
    let endpoint = std::env::var("CALYX_RERANKER_ENDPOINT")
        .unwrap_or_else(|_| "http://127.0.0.1:8089".to_string());
    assert!(
        sentinel.len() >= 16,
        "sentinel too short to be unique on disk"
    );
    fs::create_dir_all(&root).unwrap();
    let sentinel_hash = blake3::hash(sentinel.as_bytes()).to_hex().to_string();
    let client = RerankerClient::new(endpoint.clone(), Duration::from_secs(10));

    // Happy path — hand-computed expectation: the on-topic candidate must
    // outscore the off-topic one; both carry the sentinel so relative order
    // is decided by relevance, not by the sentinel tokens.
    let query = format!("what color is the daytime sky {sentinel}");
    let on_topic = format!("the daytime sky is blue {sentinel}");
    let off_topic = format!("quarterly revenue grew nine percent {sentinel}");
    let request = RerankRequest::new(query, vec![on_topic, off_topic]);
    let rendered = format!("{request:?}");
    let debug_redacts = !rendered.contains(&sentinel);
    let response = client.rerank(&request).expect("real TEI rerank");
    assert_eq!(response.scores.len(), 2);
    assert!(response.scores.iter().all(|score| score.is_finite()));
    assert!(
        response.scores[0] > response.scores[1],
        "on-topic candidate must outscore off-topic: {:?}",
        response.scores
    );
    assert!(debug_redacts, "Debug leaked the sentinel: {rendered}");

    // Edge 1 — zero candidates: fail closed before any network IO.
    let empty_err = client
        .rerank(&RerankRequest::new(sentinel.clone(), Vec::new()))
        .unwrap_err();
    assert_eq!(empty_err.code, "CALYX_SEXTANT_RERANKER_NO_CANDIDATES");

    // Edge 2 — oversized candidate (3 MB, far past the resident TEI's
    // max_input_length of 8192 tokens): the deployment accepts and
    // truncates at the token limit, so the defined outcome is exactly one
    // finite score — and the sentinel-laden megabytes still never persist.
    let oversized = sentinel.repeat(3_000_000 / sentinel.len() + 1);
    let oversized_response = client
        .rerank(&RerankRequest::new("size probe", vec![oversized]))
        .expect("resident TEI truncates oversized candidates at max_input_length");
    assert_eq!(oversized_response.scores.len(), 1);
    assert!(oversized_response.scores[0].is_finite());

    // Edge 3 — JSON-hostile candidate: quotes, backslashes, CRLF, control
    // chars, emoji. Escaping must round-trip and yield one finite score.
    let hostile = format!("\"quoted\" \\back\\slash\r\nnew\tline \u{1F512} {sentinel}");
    let hostile_response = client
        .rerank(&RerankRequest::new(
            format!("hostile probe {sentinel}"),
            vec![hostile],
        ))
        .expect("JSON-hostile candidate must be escaped, not corrupted");
    assert_eq!(hostile_response.scores.len(), 1);
    assert!(hostile_response.scores[0].is_finite());

    // Edge 4 — reranker down: fail closed with the timeout code.
    let down = RerankerClient::new("http://127.0.0.1:9", Duration::from_millis(100));
    let down_err = down
        .rerank(&RerankRequest::new("probe", vec![sentinel.clone()]))
        .unwrap_err();
    assert_eq!(down_err.code, "CALYX_SEXTANT_RERANKER_TIMEOUT");

    let evidence = json!({
        "issue": 594,
        "endpoint": endpoint,
        "sentinel_blake3": sentinel_hash,
        "sentinel_len": sentinel.len(),
        "happy_scores": response.scores,
        "happy_on_topic_outscores_off_topic": response.scores[0] > response.scores[1],
        "debug_redacts_sentinel": debug_redacts,
        "edge_empty_candidates_code": empty_err.code,
        "edge_oversized_bytes": 3_000_000,
        "edge_oversized_scores": oversized_response.scores,
        "edge_json_hostile_scores": hostile_response.scores,
        "edge_reranker_down_code": down_err.code,
    });
    fs::write(
        root.join("reranker-tei-nonpersistence-readback.json"),
        serde_json::to_vec_pretty(&evidence).unwrap(),
    )
    .unwrap();
    println!(
        "evidence={} sentinel_blake3={sentinel_hash}",
        root.join("reranker-tei-nonpersistence-readback.json")
            .display()
    );
}
