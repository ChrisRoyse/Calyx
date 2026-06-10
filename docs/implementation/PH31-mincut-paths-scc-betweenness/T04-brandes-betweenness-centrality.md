# PH31 · T04 — Brandes betweenness centrality

> **STATUS: ✅ DONE / FSV-signed-off.** Implemented in
> `crates/calyx-mincut/src/betweenness.rs` with directed Brandes centrality,
> f64 dependency accumulation, deterministic top-k, and empty-graph
> fail-closed behavior. aiwonder FSV readback:
> `ph31-betweenness-readback.json`.

> Historical checklist note: the unchecked implementation prompts below were
> satisfied by the closed Stage 6 evidence; current state is the status/evidence
> block above.

| Field | Value |
|---|---|
| **Phase** | PH31 — mincut/paths: graph build + SCC + betweenness |
| **Stage** | S6 — Lodestar Kernel |
| **Crate** | `calyx-mincut` |
| **Files** | `crates/calyx-mincut/src/betweenness.rs` (≤500) |
| **Depends on** | T03 (SCC + `CondensedGraph` available), T02 (`AssocGraph`) |
| **Axioms** | A29 |
| **PRD** | `dbprdplans/08 §3` (Stage 2: Kernel-graph — high-betweenness nodes) |

## Goal

Implement Brandes' algorithm for betweenness centrality on `AssocGraph` (directed,
weighted). Betweenness scores identify the high-centrality nodes that enter the
~10% kernel-graph in PH32. Run on the condensed graph (post-SCC) to keep `O(VE)`
tractable; accumulate in f64 to avoid catastrophic cancellation.

## Build (checklist of concrete, code-level steps)

- [ ] `pub fn betweenness(graph: &AssocGraph) -> HashMap<CxId, f64>` — Brandes
  directed betweenness; uses BFS for unweighted/unit-weight, Dijkstra for
  weighted edges; accumulates pair-dependencies in f64.
- [ ] Normalize by `(n-1)*(n-2)` for directed graphs (n = node count); single-
  node graph → all scores = 0.0.
- [ ] `pub fn betweenness_top_k(graph: &AssocGraph, k: usize) -> Vec<(CxId, f64)>`
  — returns the top-k nodes by betweenness score, descending; deterministic tie-
  breaking by `CxId` byte order.
- [ ] Optionally accept `SccResult` to skip internal SCC edges (intra-SCC
  betweenness is not meaningful for MFVS selection); controlled by a bool param.
- [ ] Error if graph has 0 nodes → `CALYX_BETWEENNESS_EMPTY_GRAPH` (f.c.).

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: path graph `A→B→C→D→E`; betweenness of `B,C,D` > `A,E`; exact values
  match a reference computation (hand-calculated or numpy for this 5-node case).
- [ ] unit: star graph `A→{B,C,D,E}` (A = hub, no back-edges); betweenness of `A`
  is maximal; `B,C,D,E` have 0.0 (no paths through them).
- [ ] unit: `betweenness_top_k(graph, 2)` on the path graph returns `[C, B]`
  or `[C, D]` (by symmetry) with the highest two scores.
- [ ] proptest: sum of all betweenness scores ≥ 0.0 for any valid random graph.
- [ ] edge: complete DAG of 3 nodes — all scores are 0.0 (every node is directly
  adjacent to every other; no node lies on a shortest path between two non-adjacent nodes).
- [ ] edge: `k > n` in `betweenness_top_k` → returns all `n` nodes without panic.
- [ ] fail-closed: empty graph → `CALYX_BETWEENNESS_EMPTY_GRAPH`; negative-weight
  edge (invalid `AssocGraph`) → panics in debug / `CALYX_GRAPH_INVALID_WEIGHT` propagated.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `cargo test -p calyx-mincut betweenness -- --nocapture` stdout on aiwonder.
- **Readback:** `cargo test -p calyx-mincut betweenness 2>&1 | tee /tmp/ph31_t04_fsv.txt && cat /tmp/ph31_t04_fsv.txt`.
- **Prove:** path-graph test prints betweenness scores matching the reference
  values to 1e-6; star-graph hub score is maximal (numerically dominant over
  leaves); output table attached to PH31 GitHub issue confirms computed vs known.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH31 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
