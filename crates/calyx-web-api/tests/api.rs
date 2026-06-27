//! Integration FSV for the calyx-web-api error envelope + resource guardrails.
//!
//! No mocks: every test drives the REAL `app()`/`build_app()` router (or the
//! REAL `guardrails`/`panic_catch_layer` middleware) in-process via
//! `tower::ServiceExt::oneshot` and inspects the actual response status +
//! JSON body + headers. Synthetic inputs with known expected outputs (an
//! oversized body, a tiny rate-limit bucket, a deliberately-slow handler, a
//! deliberately-panicking handler whose payload carries a sentinel that MUST
//! NOT appear in the response).

use std::sync::Arc;
use std::time::Duration;

use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode, header},
    middleware::from_fn_with_state,
    routing::get,
};
use calyx_web_api::{ErrorCode, Guardrails, app, build_app, guardrails, panic_catch_layer};
use http_body_util::BodyExt;
use serde_json::Value;
use tower::ServiceExt;

/// Sentinel embedded in the synthetic panic payload; it MUST NOT appear in any
/// response body (the no-leak invariant of the panic handler).
const PANIC_SENTINEL: &str = "PANIC_SENTINEL_DO_NOT_LEAK_a1b2c3";

/// A deliberately-panicking handler with a concrete return type (a bare
/// panicking closure cannot infer `IntoResponse` from the never type).
async fn boom() -> StatusCode {
    panic!("{} at /boom", PANIC_SENTINEL)
}

/// A deliberately-slow handler used to exercise the request timeout.
async fn slow() -> StatusCode {
    tokio::time::sleep(Duration::from_millis(400)).await;
    StatusCode::OK
}

/// Drive one request through a router and return (status, parsed JSON body).
async fn call(app: Router, req: Request<Body>) -> (StatusCode, Value) {
    let resp = app.oneshot(req).await.expect("router is infallible");
    let status = resp.status();
    let bytes = resp
        .into_body()
        .collect()
        .await
        .expect("collect body")
        .to_bytes();
    let json: Value = serde_json::from_slice(&bytes).expect("error responses are JSON envelopes");
    (status, json)
}

/// Assert a body is the closed `{code,message,remediation}` envelope for `code`.
fn assert_envelope(body: &Value, code: ErrorCode) {
    assert_eq!(body["code"], code.code(), "code mismatch in {body}");
    assert_eq!(
        body["remediation"],
        code.remediation(),
        "remediation mismatch"
    );
    assert!(
        body["message"].as_str().is_some_and(|m| !m.is_empty()),
        "message present"
    );
}

#[tokio::test]
async fn health_is_ok_and_not_an_error_envelope() {
    let (status, body) = call(
        app(),
        Request::get("/v1/health").body(Body::empty()).unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], "ok");
    assert_eq!(body["readOnly"], true);
    assert!(
        body.get("code").is_none(),
        "success body carries no error code"
    );
}

#[tokio::test]
async fn scaffolded_route_returns_not_implemented_envelope() {
    let (status, body) = call(
        app(),
        Request::post("/v1/measure").body(Body::empty()).unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
    assert_envelope(&body, ErrorCode::NotImplemented);
}

#[tokio::test]
async fn unknown_route_returns_not_found_envelope() {
    let (status, body) = call(
        app(),
        Request::get("/v1/does-not-exist")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_envelope(&body, ErrorCode::NotFound);
}

#[tokio::test]
async fn wrong_method_returns_method_not_allowed_envelope() {
    let (status, body) = call(
        app(),
        Request::delete("/v1/health").body(Body::empty()).unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
    assert_envelope(&body, ErrorCode::MethodNotAllowed);
}

#[tokio::test]
async fn read_only_mutating_method_on_data_route_is_405() {
    let (status, body) = call(
        app(),
        Request::delete("/v1/measure").body(Body::empty()).unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
    assert_envelope(&body, ErrorCode::MethodNotAllowed);
}

#[tokio::test]
async fn oversized_body_on_gpu_route_returns_413_envelope() {
    // GPU routes cap at MAX_GPU_BODY_BYTES (4 KiB). A 5 KiB body -> 413.
    let big = "x".repeat(5 * 1024);
    let (status, body) = call(
        app(),
        Request::post("/v1/measure").body(Body::from(big)).unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
    assert_envelope(&body, ErrorCode::PayloadTooLarge);
}

#[tokio::test]
async fn within_cap_body_passes_guardrails_to_handler() {
    // A 1 KiB body on /v1/measure is within the cap and reaches the 501 handler.
    let small = "x".repeat(1024);
    let (status, body) = call(
        app(),
        Request::post("/v1/measure")
            .body(Body::from(small))
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
    assert_envelope(&body, ErrorCode::NotImplemented);
}

#[tokio::test]
async fn rate_limit_returns_429_envelope_with_retry_after() {
    // Tiny GPU bucket: capacity 1, no refill. 1st passes, 2nd -> 429.
    let limiter = Arc::new(Guardrails::new(
        100.0,
        0.0,
        1.0,
        0.0,
        Duration::from_secs(5),
    ));
    let app = build_app(limiter);

    let r1 = app
        .clone()
        .oneshot(Request::post("/v1/guard").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(
        r1.status(),
        StatusCode::NOT_IMPLEMENTED,
        "first request consumes the token"
    );

    let resp = app
        .oneshot(Request::post("/v1/guard").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
    assert!(
        resp.headers().get(header::RETRY_AFTER).is_some(),
        "429 carries a Retry-After header"
    );
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&bytes).unwrap();
    assert_envelope(&json, ErrorCode::RateLimited);
}

#[tokio::test]
async fn slow_handler_times_out_with_504_envelope() {
    // The EXACT production guardrails middleware with a short 100ms timeout over
    // a handler that sleeps 400ms -> 504 (deterministic, fast).
    let limiter = Arc::new(Guardrails::new(
        100.0,
        100.0,
        100.0,
        100.0,
        Duration::from_millis(100),
    ));
    let app = Router::new()
        .route("/slow", get(slow))
        .layer(from_fn_with_state(limiter, guardrails));

    let (status, body) = call(app, Request::get("/slow").body(Body::empty()).unwrap()).await;
    assert_eq!(status, StatusCode::GATEWAY_TIMEOUT);
    assert_envelope(&body, ErrorCode::Timeout);
}

#[tokio::test]
async fn panic_maps_to_internal_500_envelope_and_never_leaks_detail() {
    // The EXACT production panic layer, over a synthetic panicking handler whose
    // payload carries a sentinel that must never reach the response body.
    let app = Router::new()
        .route("/boom", get(boom))
        .layer(panic_catch_layer());

    let (status, body) = call(app, Request::get("/boom").body(Body::empty()).unwrap()).await;

    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_envelope(&body, ErrorCode::Internal);
    let raw = body.to_string();
    assert!(
        !raw.contains(PANIC_SENTINEL),
        "panic sentinel leaked into response body: {raw}"
    );
    assert!(
        !raw.contains("/boom"),
        "panic location leaked into response body: {raw}"
    );
}

#[tokio::test]
async fn error_code_catalog_is_closed_unique_and_complete() {
    let mut seen = std::collections::HashSet::new();
    for code in ErrorCode::ALL {
        let wire = code.code();
        assert!(
            wire.starts_with("CALYX_WEB_API_"),
            "code {wire} must use the prefix"
        );
        assert!(
            seen.insert(wire),
            "duplicate wire code {wire} in the catalog"
        );
        assert!(
            !code.remediation().is_empty(),
            "{wire} missing a remediation"
        );
        assert!(
            !code.default_message().is_empty(),
            "{wire} missing a default message"
        );
        assert!(code.status().is_client_error() || code.status().is_server_error());
    }
    assert_eq!(seen.len(), ErrorCode::ALL.len());
}

// --- #572: MeasureCtx fail-loud config (no mocks, no silent fallback) ---

#[test]
fn measure_ctx_load_fails_loud_on_unopenable_vault() {
    let err = match calyx_web_api::MeasureCtx::load(
        std::path::Path::new("/nonexistent-calyx/01ARZ3NDEKTSV4RRFFQ69G5FAV"),
        "absent",
    ) {
        Ok(_) => panic!("an unopenable vault dir must fail loud, never silently succeed"),
        Err(e) => e,
    };
    assert!(err.contains("vault"), "error must name the failure: {err}");
}

#[test]
fn measure_ctx_load_rejects_non_vault_id_dir_name() {
    let err =
        match calyx_web_api::MeasureCtx::load(std::path::Path::new("/tmp/not-a-vault-id"), "x") {
            Ok(_) => panic!("a dir name that is not a vault id must fail loud"),
            Err(e) => e,
        };
    assert!(err.contains("not a vault id"), "got: {err}");
}

// --- #577: /v1/provenance/{id} real Ledger answer-trace (no mocks) ---
//
// Source of truth: a REAL on-disk `DirectoryLedgerStore` (row files
// `{seq:016x}.ledger`). Each test appends REAL hash-chained entries via the
// REAL `LedgerAppender`, drops the appender, RE-OPENS a fresh store on the same
// directory (proving persistence — we read what was actually written), drives
// the REAL `build_app_with_provenance` router over HTTP via `oneshot`, and
// independently re-scans the on-disk ledger to corroborate the response. Edge
// cases: unknown id, incomplete/untrusted answer, and a tampered (Broken) chain.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use calyx_core::{CxId, FixedClock};
use calyx_ledger::{
    ActorId, DirectoryLedgerStore, EntryKind, LedgerAppender, LedgerEntry, SubjectId, decode,
    encode,
};
use calyx_web_api::{ProvenanceCtx, build_app_with_provenance};

static PROV_DIR_COUNTER: AtomicU64 = AtomicU64::new(0);

/// A fresh, unique, empty ledger directory under the OS temp dir.
fn fresh_ledger_dir() -> PathBuf {
    let unique = PROV_DIR_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!(
        "calyx-web-api-prov-fsv-{}-{unique}",
        std::process::id()
    ));
    if dir.exists() {
        std::fs::remove_dir_all(&dir).expect("clear stale ledger fsv dir");
    }
    std::fs::create_dir_all(&dir).expect("create ledger fsv dir");
    dir
}

fn cx(seed: u8) -> CxId {
    CxId::from_bytes([seed; 16])
}

/// Build the provenance app over a real on-disk ledger directory.
fn prov_app(dir: &std::path::Path) -> Router {
    let ctx = ProvenanceCtx::open(dir).expect("open real on-disk ledger");
    build_app_with_provenance(Arc::new(Guardrails::production()), Arc::new(ctx))
}

/// Append the canonical complete answer trace (Kernel seq0, Guard seq1, Answer
/// seq2) to a real on-disk ledger and return the directory. `answer_id` is the
/// `Query` subject the answer is recorded under (what `{id}` matches).
fn seed_complete_answer(answer_id: &[u8]) -> PathBuf {
    let dir = fresh_ledger_dir();
    let store = DirectoryLedgerStore::open(&dir).expect("open store for seeding");
    let mut appender = LedgerAppender::open(store, FixedClock::new(1_000)).expect("open appender");
    let append = |appender: &mut LedgerAppender<DirectoryLedgerStore, FixedClock>,
                  kind: EntryKind,
                  subject: SubjectId,
                  value: serde_json::Value| {
        appender
            .append(
                kind,
                subject,
                serde_json::to_vec(&value).unwrap(),
                ActorId::Service("prov-fsv".to_string()),
            )
            .expect("append real ledger entry");
    };
    append(
        &mut appender,
        EntryKind::Kernel,
        SubjectId::Kernel(cx(88).as_bytes().to_vec()),
        serde_json::json!({"kernel_id": cx(88).to_string(), "recall_ratio": 0.99}),
    );
    append(
        &mut appender,
        EntryKind::Guard,
        SubjectId::Guard(b"guard-fsv".to_vec()),
        serde_json::json!({"guard_id": "guard-fsv", "pass": true, "tau": 0.8}),
    );
    append(
        &mut appender,
        EntryKind::Answer,
        SubjectId::Query(answer_id.to_vec()),
        serde_json::json!({
            "complete": true,
            "kernel_id": cx(88).to_string(),
            "guard_id": "guard-fsv",
            "path": []
        }),
    );
    // Drop the appender; the rows are durably on disk. The endpoint RE-OPENS the
    // directory, so what we read back is the persisted source of truth.
    drop(appender.into_store());
    dir
}

/// Count the real row files actually on disk (independent source-of-truth read).
fn on_disk_row_count(dir: &std::path::Path) -> usize {
    std::fs::read_dir(dir)
        .expect("read ledger dir")
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().extension().and_then(|e| e.to_str()) == Some("ledger"))
        .count()
}

#[tokio::test]
async fn provenance_happy_path_returns_trusted_trace_matching_on_disk_ledger() {
    let answer_id = b"ans-fsv-happy-001";
    let dir = seed_complete_answer(answer_id);

    // BEFORE: prove the source of truth physically exists on disk.
    assert_eq!(
        on_disk_row_count(&dir),
        3,
        "3 real row files must be on disk (kernel, guard, answer)"
    );

    let (status, body) = call(
        prov_app(&dir),
        Request::get(format!(
            "/v1/provenance/{}",
            std::str::from_utf8(answer_id).unwrap()
        ))
        .body(Body::empty())
        .unwrap(),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["found"], true, "answer entry found: {body}");
    assert_eq!(
        body["trusted"], true,
        "complete trace + intact chain: {body}"
    );
    assert_eq!(body["complete"], true);
    assert_eq!(body["chain"]["result"], "intact");
    assert_eq!(body["chain"]["count"], 3);
    let entries = body["entries"].as_array().expect("entries array");
    assert_eq!(entries.len(), 3, "answer + linked kernel + guard: {body}");
    // The answer entry (seq 2) must carry the real Query subject + a real BLAKE3
    // entry hash (64 hex chars), proving this is the committed ledger entry.
    let answer = entries
        .iter()
        .find(|e| e["seq"] == 2)
        .expect("answer entry seq 2");
    assert_eq!(answer["kind"], "Answer");
    assert_eq!(
        answer["entryHash"].as_str().map(|h| h.len()),
        Some(64),
        "entryHash is a 32-byte BLAKE3 hash in hex"
    );
    assert!(
        body["warnings"].as_array().unwrap().is_empty(),
        "no warnings: {body}"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn provenance_unknown_id_is_structured_not_found_not_a_500() {
    let dir = seed_complete_answer(b"ans-fsv-present");

    let (status, body) = call(
        prov_app(&dir),
        Request::get("/v1/provenance/ans-fsv-DOES-NOT-EXIST")
            .body(Body::empty())
            .unwrap(),
    )
    .await;

    assert_eq!(
        status,
        StatusCode::OK,
        "unknown id is graceful 200, never 500"
    );
    assert_eq!(body["found"], false, "no answer entry for this id: {body}");
    assert_eq!(body["trusted"], false);
    assert!(body["entries"].as_array().unwrap().is_empty());
    // The chain is still intact even though the queried id is absent.
    assert_eq!(body["chain"]["result"], "intact");

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn provenance_incomplete_answer_is_flagged_untrusted() {
    let dir = fresh_ledger_dir();
    let answer_id = b"ans-fsv-incomplete";
    {
        let store = DirectoryLedgerStore::open(&dir).unwrap();
        let mut appender = LedgerAppender::open(store, FixedClock::new(2_000)).unwrap();
        // No "complete":true marker -> the engine flags it unprovenanced.
        appender
            .append(
                EntryKind::Answer,
                SubjectId::Query(answer_id.to_vec()),
                serde_json::to_vec(&serde_json::json!({"path": []})).unwrap(),
                ActorId::Service("prov-fsv".to_string()),
            )
            .unwrap();
        drop(appender.into_store());
    }

    let (status, body) = call(
        prov_app(&dir),
        Request::get("/v1/provenance/ans-fsv-incomplete")
            .body(Body::empty())
            .unwrap(),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["found"], true, "the answer entry exists: {body}");
    assert_eq!(
        body["complete"], false,
        "but it is not marked complete: {body}"
    );
    assert_eq!(body["trusted"], false, "incomplete -> untrusted: {body}");
    assert!(
        !body["warnings"].as_array().unwrap().is_empty(),
        "an incomplete trace carries a warning: {body}"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn provenance_surfaces_broken_chain_and_forces_untrusted() {
    let answer_id = b"ans-fsv-tampered";
    let dir = seed_complete_answer(answer_id);

    // EDGE: tamper the on-disk ledger. Re-encode entry seq 1 (Guard) with a
    // changed payload but its OWN recomputed (self-valid) hash, then overwrite
    // its row file. Entry seq 2's prev_hash still points at the ORIGINAL seq-1
    // hash, so verify_chain detects a Broken chain at seq 2 — while every row
    // still decodes (a hash-chain break, not a decode corruption).
    let row1_path = dir.join(format!("{:016x}.ledger", 1));
    let original = decode(&std::fs::read(&row1_path).unwrap()).unwrap();
    let tampered = LedgerEntry::new(
        original.seq,
        original.prev_hash,
        original.kind,
        original.subject.clone(),
        serde_json::to_vec(&serde_json::json!({"guard_id": "guard-fsv", "pass": false})).unwrap(),
        original.actor.clone(),
        original.ts,
    );
    assert_ne!(
        tampered.entry_hash, original.entry_hash,
        "tampering the payload must change the self-hash"
    );
    std::fs::write(&row1_path, encode(&tampered)).expect("overwrite tampered row on disk");

    let (status, body) = call(
        prov_app(&dir),
        Request::get("/v1/provenance/ans-fsv-tampered")
            .body(Body::empty())
            .unwrap(),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["chain"]["result"], "broken",
        "verify_chain must expose the tamper: {body}"
    );
    assert_eq!(body["chain"]["atSeq"], 2, "break detected at seq 2: {body}");
    assert_eq!(
        body["trusted"], false,
        "a broken chain forces trusted=false even if the trace payload claims complete: {body}"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn provenance_is_read_only_post_is_405() {
    let dir = seed_complete_answer(b"ans-fsv-ro");
    let (status, body) = call(
        prov_app(&dir),
        Request::post("/v1/provenance/ans-fsv-ro")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
    assert_envelope(&body, ErrorCode::MethodNotAllowed);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn provenance_ctx_open_fails_loud_on_unreadable_ledger_path() {
    // A path that exists as a FILE (not a dir) cannot host a ledger CF -> fail loud.
    let file = std::env::temp_dir().join(format!(
        "calyx-web-api-prov-not-a-dir-{}",
        std::process::id()
    ));
    std::fs::write(&file, b"x").unwrap();
    let result = ProvenanceCtx::open(&file);
    std::fs::remove_file(&file).ok();
    assert!(
        result.is_err(),
        "opening a ledger over a non-directory path must fail loud, not silently succeed"
    );
}
