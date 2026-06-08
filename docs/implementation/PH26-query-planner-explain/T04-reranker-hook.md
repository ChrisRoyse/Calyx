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
ONNX cross-encoder is the embedded fallback. Candidate text handling is
request-scoped, serialized request bytes are held in `Zeroizing<String>`, and
candidate text is never written to WAL, disk, or any log. Hard timeout of
`rerank_timeout_ms` (default 5000ms); fail-closed on timeout or HTTP error.

**Current implementation note (#290):** public `RerankRequest` keeps
`query`/`candidates` for Calyx callers, while the HTTP wire request serializes
to TEI's actual `{ "query": ..., "texts": [...] }` shape. TEI returns
`[{ "index": usize, "score": f32 }]`; Calyx maps those rank entries back into
candidate order and rejects non-2xx, malformed, duplicate, non-finite, or
incomplete responses with `CALYX_SEXTANT_RERANKER_TIMEOUT`.

## Build (checklist of concrete, code-level steps)

- [ ] `crates/calyx-sextant/src/reranker.rs`:
  ```rust
  pub struct RerankerClient {
      pub endpoint: String,       // e.g. "http://127.0.0.1:8089/rerank"
      pub timeout_ms: u64,        // default 5000
  }

  pub struct RerankRequest {
      pub query: String,
      pub candidates: Vec<String>,
  }

  pub struct RerankResponse {
      pub scores: Vec<f32>,  // candidate-order scores
      pub zeroizing_ok: bool,
  }
  ```
- [ ] `fn rerank(&self, req: RerankRequest) -> Result<RerankResponse, CalyxError>`:
      - Serialize `{ "query": ..., "texts": [...] }` as JSON
      - POST to `self.endpoint` with `Content-Type: application/json`
      - `timeout(Duration::from_millis(self.timeout_ms))`
      - On HTTP error or timeout → `CALYX_SEXTANT_RERANKER_TIMEOUT` (covers both)
      - Parse TEI response `[{ "index": 0, "score": 0.5 }, ...]` into
        candidate-order scores
      - Serialized request body is scoped through a `Zeroizing` value; candidate
        strings are borrowed from the caller and never persisted or logged
- [ ] Wire into `PipelineStrategy` (replace the stub from PH25 T05)
- [ ] `RerankerClient::new_local()` → creates a client pointed at `127.0.0.1:8089`
- [ ] malformed JSON/shape → `CALYX_SEXTANT_RERANKER_TIMEOUT`

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: serialized request JSON has `"query"` and `"texts"` keys — assert
      using `serde_json::from_str` on the expected shape
- [ ] unit (mock): spin up a `tiny_http` mock server in the test that returns
      `[{"index":0,"score":0.9},{"index":1,"score":0.5}]` → assert
      `RerankResponse` has correct candidate-order scores
- [ ] edge (mock): mock server returns 500 → `CALYX_SEXTANT_RERANKER_TIMEOUT`
- [ ] edge (mock): mock server sleeps > timeout → `CALYX_SEXTANT_RERANKER_TIMEOUT`
- [ ] edge: empty candidate list → `Ok(RerankResponse { scores: vec![] })`
- [ ] fail-closed: malformed JSON from server → `CALYX_SEXTANT_RERANKER_TIMEOUT`

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** test output of `cargo test -p calyx-sextant reranker -- --nocapture`
  on aiwonder; the resident `:8089` GTE reranker is running
- **Readback:** `cargo test -p calyx-sextant stage4_full_stack_fsv -- --ignored --nocapture`
- **Prove:** Stage 4 readback includes `rerank.scores` from live `:8089`,
  `zeroizing_ok=true`, and non-2xx unit coverage for
  `CALYX_SEXTANT_RERANKER_TIMEOUT`.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH26 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
