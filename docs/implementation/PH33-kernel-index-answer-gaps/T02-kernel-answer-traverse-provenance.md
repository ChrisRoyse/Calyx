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
stamp every hop and completed answer with a durable Ledger reference. This
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
  4. Without a Ledger appender, fail every answerable path closed with
     `CALYX_KERNEL_ANSWER_LEDGER_REQUIRED`, including the zero-hop direct hit.
  5. The ledger-backed API builds the full `hops` list, appends each real hop
     reference plus the complete-answer reference, and returns `AnswerPath` with
     `total_score = Σ hop_scores`.
- [x] `max_hops` is fail-closed: if the query path exists only beyond the bound,
  return `CALYX_PATHS_MAX_HOPS` instead of a truncated `AnswerPath`.
- [x] `total_score` is finite and ≥ 0.0; NaN/Inf → `CALYX_KERNEL_SCORE_INVALID`.
- [x] Provenance: every returned `LedgerRef` is re-read from Ledger storage and
  verified for sequence, `EntryKind::Answer`, and entry hash before success.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: chain graph `K→A→B→C` where K is anchored kernel node, query_cx = C;
  `kernel_answer_with_ledger` returns path `[K→A→B→C]`; hop scores = `[0.9^0, 0.9^1, 0.9^2]`
  times edge weights; total_score correct to ε=1e-5.
- [x] unit: `kernel_answer` with `max_hops=2` on a depth-3 chain →
  `CALYX_PATHS_MAX_HOPS`; no truncated answer is returned.
- [x] unit: kernel with 0 anchored nodes → `CALYX_KERNEL_NO_ANCHORED_NODE`.
- [x] unit: every hop in `hops` has a readable, hash-matching Ledger reference,
  followed by the complete-answer reference.
- [x] edge: `query_cx` == `anchor_kernel_node` (query is already a kernel node) →
  ledger-less `kernel_answer` returns `CALYX_KERNEL_ANSWER_LEDGER_REQUIRED`;
  the ledger-backed API returns `hops = []`, `total_score = 1.0`, and appends a
  complete `kind=Answer` row with `expected_hops=0` (#647, #1380).
- [x] edge: missing query node propagates the `CALYX_PATHS_NODE_NOT_FOUND` graph error.
- [x] edge: a nearer anchored candidate with no bounded path is skipped; the next
  reachable anchored candidate is selected and produces the full answer path.
- [x] fail-closed: `total_score` becomes NaN →
  `CALYX_KERNEL_SCORE_INVALID`.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** physical Ledger row bytes under the explicit retained FSV root on
  aiwonder. Trigger output and JSON reports are diagnostic only; absence/count of
  `.ledger` rows is the verdict for fail-closed paths.
- **Readback:** use the exact retained Calyx binary to read each physical row with
  `calyx readback --hex`, then independently list/count/hash the row files and
  verify the decoded sequence, `EntryKind::Answer`, entry hash, and chain links.
- **Prove:** ledger-less direct self-hit returns
  `CALYX_KERNEL_ANSWER_LEDGER_REQUIRED` without writing a row; the ledger-backed
  direct self-hit writes a complete Answer row with `expected_hops=0` and
  `path=[]`; no-anchor, dimension-mismatch, and bounded non-self paths leave the
  Ledger byte count unchanged. Attach the bounded byte readbacks to the issue.
- **#630 real-corpus bound:** aiwonder readback root
  `/home/croyse/calyx/data/fsv-issue630-real-anchor-search-20260610` proves the
  fallback on real SciFact bytes: candidate bound `158`, old window `10`, anchor
  rank `76`, answer path `8` hops, decoded answer JSON read back from disk, and
  source hashes `28f4c3e5cdc276b03d4605ea63d3ac19` /
  `193519c60f28c755ee2252d544f5885e`. The FSV passes the full real anchored set
  through `kernel_answer_with_ledger`, not a preselected one-anchor shortcut.
- **#631 real-corpus Ledger trace:** aiwonder readback root
  `/home/croyse/calyx/data/fsv-issue631-real-ledger-answer-20260610` proves
  `kernel_answer_with_ledger` on real SciFact bytes: before ledger rows `0`,
  after rows `6`, kernel row seq `0`, hop Answer seqs `[1,2,3,4]`, complete
  Answer row seq `5`, `get_answer_trace` path length `4`, no warnings, and
  `trace_trusted=true`. `BLAKE3SUMS.txt` verifies the JSON artifacts and all
  physical `ledger-cf/*.ledger` row bytes.
- **#647 direct-hit Ledger trace:** aiwonder readback root
  `/home/croyse/calyx/data/fsv-issue647-direct-hit-ledger-20260611T073538Z`
  proves `kernel_answer_with_ledger` on `query_cx == anchor`: before ledger rows
  `0`, after rows `2`, one `kind=Kernel` row, one complete `kind=Answer` row,
  `expected_hops=0`, `path=[]`, `trace_path_len=0`, and `trace_trusted=true`.
  Primary readback `ph33-direct-hit-ledger-readback.json` SHA-256:
  `c14c9e985ed3b63ed5faba5ac91fcd6f324a8bd5f2762f7525992082ca57c9d8`.

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] FSV evidence (readback output / screenshot) attached to the PH33 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
