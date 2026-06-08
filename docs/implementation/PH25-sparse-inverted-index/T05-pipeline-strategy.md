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

**Current implementation note (#290):** `FusionStrategy::Pipeline` is implemented
through `FusionContext.stage1_slots`, which `SearchEngine` fills from
inverted/sparse slot stats. `pipeline_fuse` derives the stage-1 candidate set
from those slots and restricts final multi-lens scoring to that set. The
zero-candidate edge returns zero Pipeline hits rather than falling back to
dense-only scoring. The reranker hook is a separate `RerankerClient` step using
the live TEI `texts` wire schema; HTTP non-2xx fails closed with
`CALYX_SEXTANT_RERANKER_TIMEOUT`.

**Current implementation note (#324):** `Query::recall_k` configures the sparse
stage-1 window for Pipeline. If it is omitted, Pipeline uses `query.k * 10`.
If it is set lower than `query.k`, stage 1 limits the result count. Dense
scoring and reranker request construction run over the recall window, and final
results are truncated to `query.k` afterward.

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
- [ ] HTTP call to reranker: blocking HTTP client; timeout 5s;
      `CALYX_SEXTANT_RERANKER_TIMEOUT` on failure; fail-closed (do not return
      unranked results silently — either rerank or error)
- [x] Privacy invariant enforced in code: candidate text is wrapped as
      `Zeroizing<String>` when it leaves the sparse index and
      `RerankRequest.candidates` owns `Vec<Zeroizing<String>>`; the serialized
      HTTP body is also `Zeroizing<String>` (#325)
- [ ] Wire `FusionStrategy::Pipeline` in the dispatcher → `PipelineStrategy`

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: pipeline with `rerank=None` → returns top-k hits with correct
      `per_lens` entries from both sparse and dense stages
- [ ] unit: stage 1 candidates are a strict superset of the final top-k
      (pipeline never returns a hit that wasn't in stage 1)
- [x] unit: `recall_k=1, k=10` returns at most 1 hit (stage 1 limits candidates)
- [x] unit: `k=1, recall_k=3` can recover a dense-preferred candidate outside
      sparse top-1 while the final result length remains 1
- [ ] proptest: pipeline results are a subset of stage-1 candidates
- [ ] edge: sparse slot returns 0 candidates (no term matches) → `Ok(vec![])`
- [ ] edge: reranker endpoint unreachable → `CALYX_SEXTANT_RERANKER_TIMEOUT`,
      not a silent fallback to un-reranked results
- [ ] fail-closed: candidate text variable is `Zeroizing<String>` — assert via
      `std::mem::size_of_val` test that the type is the right newtype (not a plain
      `String`); this is a code-pattern check, not a runtime assertion

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** Stage 4 readback JSON on aiwonder.
- **Readback:** `cargo test -p calyx-sextant stage4_full_stack_fsv -- --ignored --nocapture`
- **Prove:** readback contains `pipeline_subset_ok=true`, `pipeline_hits>0`,
  `pipeline_empty_stage1_hits=0`, real `rerank.scores`, and
  `zeroizing_ok=true`.
- **Post-sweep #324 SoT:**
  `/home/croyse/calyx/data/fsv-issue324-pipeline-recall-headroom-20260608/pipeline-recall-headroom-readback.json`
  proves `recovered_outside_sparse_top_k=true`, `wide_final_len=1`,
  `reranker_request_text_count=3`, and `reranked_final_len=1`.
- **Post-sweep #325 SoT:**
  `/home/croyse/calyx/data/fsv-issue325-reranker-candidate-privacy-20260608/reranker-search-readback.json`
  proves `candidates_owned_by_zeroizing=true` and
  `serialized_body_zeroizing=true`.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH25 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
