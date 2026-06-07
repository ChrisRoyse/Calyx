# PH55 · T04 — `ASK`: multi-lens + `kernel_answer` + Oracle + provenance tag

| Field | Value |
|---|---|
| **Phase** | PH55 — Cross-model transactions + universal query surface |
| **Stage** | S12 — Universal data layer |
| **Crate** | `calyx-sextant` |
| **Files** | `crates/calyx-sextant/src/query/ask.rs` (≤500) |
| **Depends on** | T03 (executor context + ProvenancedRow), PH33 (kernel_answer), PH24 (multi-lens RRF), PH35 (LedgerRef provenance stub) |
| **Axioms** | A15, A16, A17, A19 |
| **PRD** | `dbprdplans/20 §2` (ASK row), `dbprdplans/10 §0`, `dbprdplans/17 §7.3` |

## Goal

Implement the `ASK` query mode: given a natural-language question and a set of
candidate `CxId`s from prior pipeline steps, run multi-lens RRF retrieval
(from PH24) to rank the most relevant constellations, pass the top-K to
`kernel_answer` (PH33), and ground the answer through the Oracle (PH49 — stub
if not yet built). Every returned result is tagged with a `LedgerRef`
(provenance chain entry). `ASK = multi-lens + kernel_answer + Oracle, grounded
+ provenanced` per `20 §2`.

## Build (checklist of concrete, code-level steps)

- [ ] Define `AskSpec` in `query/mod.rs`:
  ```rust
  pub struct AskSpec {
      pub question: String,
      pub context_cx_ids: Vec<CxId>,   // candidate set from prior steps (may be empty = full vault)
      pub top_k: usize,                // default 10
      pub oracle: bool,                // whether to run Oracle grounding (PH49; stub if absent)
  }
  ```
- [ ] Implement `ask(vault: &AsterVault, spec: &AskSpec, snapshot_seq: Seq) -> Result<AskResult>`:
  1. **Multi-lens retrieval:** embed `spec.question` using the vault's default
     panel (call `registry::embed_query`); run `sextant::fuse_rrf` restricted
     to `spec.context_cx_ids` (or full vault if empty); return top-`top_k`
     `(CxId, score)` pairs.
  2. **Kernel answer:** call `lodestar::kernel_answer(vault, question, top_cx_ids,
     snapshot_seq)` (PH33 API); returns `KernelAnswer { text, grounding_cx_ids,
     gaps: Vec<String> }`. If PH33 not yet built: stub returns `text="[kernel
     stub]", grounding_cx_ids=top_cx_ids, gaps=[]`.
  3. **Oracle grounding** (if `spec.oracle=true`): call
     `oracle::consequence_predict(vault, kernel_answer)` (PH49 API). Stub: wrap
     `kernel_answer.text` as an `OracleResult` with `conf=None`.
  4. **Provenance:** for each `grounding_cx_id`, look up its `LedgerRef` from the
     `ledger` CF at `snapshot_seq`; attach to the result.
  5. Return `AskResult { answer: String, grounding: Vec<ProvenancedRow>,
     gaps: Vec<String>, oracle_conf: Option<f32> }`.
- [ ] Enforce: if `spec.question` is empty → `CALYX_INVALID_ARGUMENT`.
- [ ] Enforce: answer must not be returned without at least one `grounding_cx_id`
  (even the stub must produce ≥1 grounding entry). If `kernel_answer` returns
  empty grounding → return `CALYX_ANSWER_UNGROUNDED` (A16 fail-closed).
- [ ] The `ASK` step in the executor pipeline calls `ask(...)` and appends
  `AskResult.grounding` rows to the `QueryResult.rows` with their `ledger_ref`s.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit stub path: `ask` with `oracle=false`, stubbed `kernel_answer` → returns
  `AskResult` with `answer="[kernel stub]"`, `grounding.len() >= 1`, each
  grounding row has `ledger_ref=Some(…)` or `None` (both acceptable for stub).
- [ ] unit provenance tag: ingest a real constellation with a Ledger stub (PH35
  stub); `ask` with that `CxId` in context → returned grounding row has
  `ledger_ref=Some(ref)` with the correct `seq`.
- [ ] unit empty context: `ask` with empty `context_cx_ids` → retrieval searches
  full vault; returns ≥0 rows (empty vault OK).
- [ ] edge (≥3): (1) empty `question` → `CALYX_INVALID_ARGUMENT`; (2)
  `kernel_answer` stub returns empty grounding → `CALYX_ANSWER_UNGROUNDED`;
  (3) `top_k=1` → exactly 1 grounding row (no more); (4) `oracle=false` →
  `oracle_conf=None`.
- [ ] fail-closed: `registry::embed_query` returns `Err` (lens unavailable) →
  `ask` returns `CALYX_LENS_NOT_FOUND`, not a silent empty result.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `cargo test -p calyx-sextant query::ask` on aiwonder.
- **Readback:**
  ```
  calyx ask --vault /home/croyse/calyx/test-vault "What orders were placed recently?" \
    --provenance --top-k 3
  # Output must show:
  # answer: <non-empty string>
  # grounding[0].ledger_ref: <hex seq>
  # grounding[0].cx_id: <hex>
  ```
- **Prove:** `calyx ask` returns a non-empty answer with ≥1 grounding row and
  a `ledger_ref` field (even if stub value); `--provenance` flag prints the
  `LedgerRef` hex. Evidence posted to PH55 issue.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH55 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
