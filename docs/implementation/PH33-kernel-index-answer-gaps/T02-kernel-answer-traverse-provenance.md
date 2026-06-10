# PH33 · T02 — `kernel_answer`: ground → traverse association edges → provenance

| Field | Value |
|---|---|
| **Phase** | PH33 — Kernel index + kernel_answer + grounding_gaps |
| **Stage** | S6 — Lodestar Kernel |
| **Crate** | `calyx-lodestar` |
| **Files** | `crates/calyx-lodestar/src/kernel_answer.rs` (≤500) |
| **Depends on** | T01 (`kernel_search`, `KernelIndex`), PH31-T01 (`reach` bounded by `max_hops`) |
| **Axioms** | A10, A11, A15 |
| **PRD** | `dbprdplans/08 §4.2`, `08 §8` |

## Goal

Implement `kernel_answer`: given a query, (1) find the nearest **answerable
anchored** kernel node via an exhaustive kernel-index candidate scan bounded by
`index.rows().len()`, continuing past unreachable anchored candidates, (2)
traverse association edges from that kernel node toward the query region using
bounded `reach`, (3) compose the answer path with `0.9^hop` hop-attenuation and
stamp every hop with a provenance reference (Ledger stub until PH35). This
implements the "retrieval that reasons over the grounded skeleton" from
`08 §4.2`.

## Build (checklist of concrete, code-level steps)

- [x] `pub struct AnswerPath { query_cx: CxId, anchor_kernel_node: CxId, hops: Vec<AnswerHop>, total_score: f32, provenance: Vec<LedgerRef> }`.
- [x] `pub struct AnswerHop { from: CxId, to: CxId, edge_weight: f32, hop_score: f32, ledger_ref: LedgerRef }` — `hop_score = edge_weight * 0.9^hop_index`.
- [x] `pub fn kernel_answer(kernel_index: &KernelIndex, graph: &AssocGraph, query_cx: CxId, query_vec: &[f32], anchored_kernel_nodes: &[CxId], max_hops: usize) -> Result<AnswerPath, CalyxError>`:
  1. `kernel_search(query_vec, top_k=index.rows().len())` → exhaustive candidate
     scan over the current kernel index, not a fixed top-10 window.
  2. Filter to supplied anchored kernel nodes in rank order; validate each
     candidate with bounded `reach(graph, kernel_node, query_cx, max_hops)`.
  3. Return the first anchored candidate with a valid bounded path; if no
     anchored candidate can answer, fail closed with the no-anchor/no-path/max-
     hops error instead of returning a truncated or ungrounded answer.
  4. Build `hops` list from the full bounded path; `ledger_ref` is a deterministic stub until PH35.
  5. Return `AnswerPath` with all hops and `total_score = Σ hop_scores`.
- [x] `max_hops` is fail-closed: if the query path exists only beyond the bound,
  return `CALYX_PATHS_MAX_HOPS` instead of a truncated `AnswerPath`.
- [x] `total_score` is finite and ≥ 0.0; NaN/Inf → `CALYX_KERNEL_SCORE_INVALID`.
- [x] Provenance: each stub `LedgerRef` hashes `(src_cx, dst_cx, hop_index)` and uses `seq = hop_index + 1`
  so PH35 can back-fill real entries.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: chain graph `K→A→B→C` where K is anchored kernel node, query_cx = C;
  `kernel_answer` returns path `[K→A→B→C]`; hop scores = `[0.9^0, 0.9^1, 0.9^2]`
  times edge weights; total_score correct to ε=1e-5.
- [x] unit: `kernel_answer` with `max_hops=2` on a depth-3 chain →
  `CALYX_PATHS_MAX_HOPS`; no truncated answer is returned.
- [x] unit: kernel with 0 anchored nodes → `CALYX_KERNEL_NO_ANCHORED_NODE`.
- [x] unit: every hop in `hops` has a non-None `ledger_ref` (even the stub) — stub
  carries non-zero `hop_index` field.
- [x] edge: `query_cx` == `anchor_kernel_node` (query is already a kernel node) →
  `hops = []`; `total_score = 1.0`.
- [x] edge: missing query node propagates the `CALYX_PATHS_NODE_NOT_FOUND` graph error.
- [x] edge: a nearer anchored candidate with no bounded path is skipped; the next
  reachable anchored candidate is selected and produces the full answer path.
- [x] fail-closed: `total_score` becomes NaN →
  `CALYX_KERNEL_SCORE_INVALID`.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** PH33 T02 JSON readback files under the explicit `CALYX_FSV_ROOT` on
  aiwonder, plus the test stdout that names each file.
- **Readback:** run `CALYX_FSV_ROOT=/home/croyse/calyx/data/fsv-issue292-kernel-answer-max-hops-20260608 cargo test -p calyx-lodestar kernel_answer -- --nocapture`,
  then separately `cat` `chain/kernel-answer-chain.json`,
  `edges/kernel-answer-max-hops.json`, and `edges/kernel-answer-errors.json`.
- **Prove:** chain test prints hop scores `[1.0, 0.9, 0.81]` (unit-weight edges);
  max-hop test prints `CALYX_PATHS_MAX_HOPS` and no `AnswerPath` prefix;
  no-anchor test prints `CALYX_KERNEL_NO_ANCHORED_NODE`; all hops show non-None
  ledger_ref stubs; output attached to the PH33 GitHub issue.
- **#630 real-corpus bound:** aiwonder readback root
  `/home/croyse/calyx/data/fsv-issue630-real-anchor-search-20260610` proves the
  fallback on real SciFact bytes: candidate bound `158`, old window `10`, anchor
  rank `76`, answer path `8` hops, decoded answer JSON read back from disk, and
  source hashes `28f4c3e5cdc276b03d4605ea63d3ac19` /
  `193519c60f28c755ee2252d544f5885e`. The FSV passes the full real anchored set
  through `kernel_answer`, not a preselected one-anchor shortcut.

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] FSV evidence (readback output / screenshot) attached to the PH33 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
