# PH25 · T05 — `Pipeline` strategy (sparse recall → multi-lens score → rerank hook)

| Field | Value |
|---|---|
| **Phase** | PH25 — Sparse lens inverted index |
| **Stage** | S4 — Sextant Search & Navigation |
| **Crate** | `calyx-sextant` |
| **Files** | `crates/calyx-sextant/src/fusion/pipeline.rs` (≤500) |
| **Depends on** | T04 (this phase) · PH24 T03 (RRF), T05 (provenance) |
| **Axioms** | A16, A17 |
| **PRD** | `dbprdplans/10 §2`, `dbprdplans/10 §7` |

## Goal

Implement the `Pipeline` fusion strategy: stage 1 uses the sparse (BM25) slot
to recall a candidate set; stage 2 scores those candidates with multi-lens RRF;
stage 3 optionally reranks via the `:8089` GTE reranker (candidate text
request-scoped and never persisted — privacy requirement, `10 §7`). This is the
maximum-precision path for the ContextGraph `E13→E1→E12` pattern.

## Build (checklist of concrete, code-level steps)

- [ ] `PipelineStrategy` struct:
  ```rust
  pub struct PipelineStrategy {
      pub sparse_slot: SlotId,          // stage 1 recall
      pub dense_slots: Vec<SlotId>,     // stage 2 multi-lens RRF
      pub recall_k: usize,              // candidates from stage 1 (default: k * 10)
      pub rerank: Option<RerankSpec>,   // stage 3 (optional)
      pub rrf_config: Bm25Config,       // BM25 params for sparse recall
  }
  ```
- [ ] `fn fuse(&self, ctx: &FusionContext) -> Result<Vec<Hit>, CalyxError>`:
      Stage 1: `sparse_slot.search(query, recall_k, ef=0)` → candidate `CxId` set
      Stage 2: for each candidate, compute multi-lens RRF score using only the
               candidate subset (not the full index — this is the efficiency win);
               build `Hit` per candidate
      Stage 3: if `rerank` is `Some(spec)`, call the reranker HTTP endpoint
               `spec.endpoint` (`:8089` on aiwonder) with candidate texts;
               the reranker receives `(query_text, candidate_text)` pairs,
               returns reranked scores; update `hit.fused_score` with reranker
               score; candidate texts are request-scoped — zero them from memory
               after the HTTP call returns; never write to disk or WAL
- [ ] HTTP call to reranker: use `ureq` (blocking, no async dep); timeout 5s;
      `CALYX_SEXTANT_RERANKER_TIMEOUT` on failure; fail-closed (do not return
      unranked results silently — either rerank or error)
- [ ] Privacy invariant enforced in code: `candidate_text` is a local variable
      in the pipeline function scope; zeroize on drop via `zeroize::Zeroizing`
- [ ] Wire `FusionStrategy::Pipeline` in the dispatcher → `PipelineStrategy`

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: pipeline with `rerank=None` → returns top-k hits with correct
      `per_lens` entries from both sparse and dense stages
- [ ] unit: stage 1 candidates are a strict superset of the final top-k
      (pipeline never returns a hit that wasn't in stage 1)
- [ ] unit: `recall_k=1, k=10` → returns at most 1 hit (stage 1 limits candidates)
- [ ] proptest: pipeline results are a subset of stage-1 candidates
- [ ] edge: sparse slot returns 0 candidates (no term matches) → `Ok(vec![])`
- [ ] edge: reranker endpoint unreachable → `CALYX_SEXTANT_RERANKER_TIMEOUT`,
      not a silent fallback to un-reranked results
- [ ] fail-closed: candidate text variable is `Zeroizing<String>` — assert via
      `std::mem::size_of_val` test that the type is the right newtype (not a plain
      `String`); this is a code-pattern check, not a runtime assertion

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** test output of `cargo test -p calyx-sextant pipeline -- --nocapture`
- **Readback:** `cargo test -p calyx-sextant pipeline -- --nocapture 2>&1`
- **Prove:** test without reranker prints `strategy=pipeline stage1_candidates=N
  final_hits=K subset_ok=true`; privacy type check prints `zeroizing_ok=true`

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH25 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
