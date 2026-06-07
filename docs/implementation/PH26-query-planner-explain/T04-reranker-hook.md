# PH26 · T04 — Reranker hook (`:8089`, Zeroizing, timeout)

| Field | Value |
|---|---|
| **Phase** | PH26 — Query planner + intent + explain |
| **Stage** | S4 — Sextant Search & Navigation |
| **Crate** | `calyx-sextant` |
| **Files** | `crates/calyx-sextant/src/reranker.rs` (≤500) |
| **Depends on** | T03 (this phase) · PH25 T05 (Pipeline reranker stub) |
| **Axioms** | A16, A17 |
| **PRD** | `dbprdplans/10 §7` |

## Goal

A production-quality reranker HTTP client that replaces the stub in PH25 T05.
The GTE cross-encoder reranker at `:8089` on aiwonder is the resident model;
ONNX cross-encoder is the embedded fallback. Candidate texts are request-scoped
(`Zeroizing<String>`) and never written to WAL, disk, or any log. Hard timeout
of `rerank_timeout_ms` (default 5000ms); fail-closed on timeout or HTTP error.

## Build (checklist of concrete, code-level steps)

- [ ] `crates/calyx-sextant/src/reranker.rs`:
  ```rust
  pub struct RerankerClient {
      pub endpoint: String,       // e.g. "http://127.0.0.1:8089/rerank"
      pub timeout_ms: u64,        // default 5000
  }

  pub struct RerankRequest {
      pub query: Zeroizing<String>,
      pub candidates: Vec<(CxId, Zeroizing<String>)>,  // (id, text)
  }

  pub struct RerankResponse {
      pub scores: Vec<(CxId, f32)>,  // reranked order
  }
  ```
- [ ] `fn rerank(&self, req: RerankRequest) -> Result<RerankResponse, CalyxError>`:
      - Serialize `{ "query": ..., "texts": [...] }` as JSON
      - POST to `self.endpoint` with `Content-Type: application/json`
      - `timeout(Duration::from_millis(self.timeout_ms))`
      - On HTTP error or timeout → `CALYX_SEXTANT_RERANKER_TIMEOUT` (covers both)
      - Parse response `{ "scores": [[cx_id_str, score], ...] }` into `RerankResponse`
      - `Zeroizing<String>` fields are dropped immediately after serialization;
        the serialized bytes are in a `Zeroizing<Vec<u8>>` on the stack
      - After response is parsed, the raw HTTP body bytes are also zeroized
- [ ] Wire into `PipelineStrategy` (replace the stub from PH25 T05)
- [ ] `RerankerClient::new_local()` → creates a client pointed at `127.0.0.1:8089`
- [ ] `CALYX_SEXTANT_RERANKER_PARSE_ERROR` for malformed JSON response

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: `RerankRequest` field types are `Zeroizing<String>` — assert at
      compile time via `TypeId` or by calling `.zeroize()` explicitly in a test
- [ ] unit: serialized request JSON has `"query"` and `"texts"` keys — assert
      using `serde_json::from_str` on the expected shape
- [ ] unit (mock): spin up a `tiny_http` mock server in the test that returns
      `{"scores": [["cx0", 0.9], ["cx1", 0.5]]}` → assert `RerankResponse` has
      correct scores and CxId ordering
- [ ] edge (mock): mock server returns 500 → `CALYX_SEXTANT_RERANKER_TIMEOUT`
- [ ] edge (mock): mock server sleeps > timeout → `CALYX_SEXTANT_RERANKER_TIMEOUT`
- [ ] edge: empty candidate list → `Ok(RerankResponse { scores: vec![] })`
- [ ] fail-closed: malformed JSON from server → `CALYX_SEXTANT_RERANKER_PARSE_ERROR`

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** test output of `cargo test -p calyx-sextant reranker -- --nocapture`
  on aiwonder; the resident `:8089` GTE reranker is running
- **Readback:** `cargo test -p calyx-sextant reranker -- --nocapture 2>&1 | grep -E 'rerank|zeroizing|timeout'`
- **Prove:** integration test against the live `:8089` endpoint (marked `#[ignore]`,
  run explicitly for FSV): prints `rerank_ok=true scores_len=N` with N matching
  the candidate count; the mock-server tests (always run) print
  `mock_ok=true timeout_ok=true zeroizing_ok=true`

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH26 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
