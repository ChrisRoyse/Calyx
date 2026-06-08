# PH33 · T02 — `kernel_answer`: ground → traverse association edges → provenance

| Field | Value |
|---|---|
| **Phase** | PH33 — Kernel index + kernel_answer + grounding_gaps |
| **Stage** | S6 — Lodestar Kernel |
| **Crate** | `calyx-lodestar` |
| **Files** | `crates/calyx-lodestar/src/kernel_answer.rs` (≤500) |
| **Depends on** | T01 (`kernel_search`, `KernelIndex`), PH31-T01 (`reach_scored` with hop-attenuation) |
| **Axioms** | A10, A11, A15 |
| **PRD** | `dbprdplans/08 §4.2`, `08 §8` |

## Goal

Implement `kernel_answer`: given a query, (1) find the nearest **anchored** kernel
node via `kernel_search`, (2) traverse association edges from that kernel node
toward the query region using `reach_scored` with `0.9^hop` hop-attenuation, (3)
compose the answer path and stamp every hop with a provenance reference (Ledger
stub until PH35). This implements the "retrieval that reasons over the grounded
skeleton" from `08 §4.2`.

## Build (checklist of concrete, code-level steps)

- [x] `pub struct AnswerPath { query_cx: CxId, anchor_kernel_node: CxId, hops: Vec<AnswerHop>, total_score: f32, provenance: Vec<LedgerRef> }`.
- [x] `pub struct AnswerHop { from: CxId, to: CxId, edge_weight: f32, hop_score: f32, ledger_ref: LedgerRef }` — `hop_score = edge_weight * 0.9^hop_index`.
- [x] `pub fn kernel_answer(kernel_index: &KernelIndex, graph: &AssocGraph, query_cx: CxId, query_vec: &[f32], anchored_kernel_nodes: &[CxId], max_hops: usize) -> Result<AnswerPath, CalyxError>`:
  1. `kernel_search(query_vec, top_k=10)` → candidate kernel nodes.
  2. Filter to supplied anchored kernel nodes; if none found → `CALYX_KERNEL_NO_ANCHORED_NODE`.
  3. From the top anchored kernel node, validate reachability with `reach_scored(graph, kernel_node, max_hops)`.
  4. Build `hops` list from the path prefix; `ledger_ref` is a deterministic stub until PH35.
  5. Return `AnswerPath` with all hops and `total_score = Σ hop_scores`.
- [x] `total_score` is finite and ≥ 0.0; NaN/Inf → `CALYX_KERNEL_SCORE_INVALID`.
- [x] Provenance: each stub `LedgerRef` hashes `(src_cx, dst_cx, hop_index)` and uses `seq = hop_index + 1`
  so PH35 can back-fill real entries.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: chain graph `K→A→B→C` where K is anchored kernel node, query_cx = C;
  `kernel_answer` returns path `[K→A→B→C]`; hop scores = `[0.9^0, 0.9^1, 0.9^2]`
  times edge weights; total_score correct to ε=1e-5.
- [x] unit: `kernel_answer` with `max_hops=2` on a depth-3 chain → stops at depth 2;
  `hops.len() == 2`.
- [x] unit: kernel with 0 anchored nodes → `CALYX_KERNEL_NO_ANCHORED_NODE`.
- [x] unit: every hop in `hops` has a non-None `ledger_ref` (even the stub) — stub
  carries non-zero `hop_index` field.
- [x] edge: `query_cx` == `anchor_kernel_node` (query is already a kernel node) →
  `hops = []`; `total_score = 1.0`.
- [x] edge: missing query node propagates the `CALYX_PATHS_NODE_NOT_FOUND` graph error.
- [x] fail-closed: `total_score` becomes NaN →
  `CALYX_KERNEL_SCORE_INVALID`.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `cargo test -p calyx-lodestar kernel_answer -- --nocapture` stdout.
- **Readback:** `cargo test -p calyx-lodestar kernel_answer 2>&1 | tee /tmp/ph33_t02_fsv.txt && cat /tmp/ph33_t02_fsv.txt`.
- **Prove:** chain test prints hop scores `[1.0, 0.9, 0.81]` (unit-weight edges);
  no-anchor test prints `CALYX_KERNEL_NO_ANCHORED_NODE`; all hops show non-None
  ledger_ref stubs; output attached to PH33 GitHub issue.

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] FSV evidence (readback output / screenshot) attached to the PH33 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
