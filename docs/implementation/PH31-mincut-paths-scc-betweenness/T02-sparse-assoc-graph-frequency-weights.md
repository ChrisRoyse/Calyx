# PH31 · T02 — Sparse AssocGraph with frequency-weighted nodes

| Field | Value |
|---|---|
| **Phase** | PH31 — mincut/paths: graph build + SCC + betweenness |
| **Stage** | S6 — Lodestar Kernel |
| **Crate** | `calyx-paths` |
| **Files** | `crates/calyx-paths/src/graph.rs` (≤500) |
| **Depends on** | T01 (lib.rs + traversal in place) |
| **Axioms** | A29 |
| **PRD** | `dbprdplans/08 §2` |

## Goal

Implement `AssocGraph`: the CSR-style sparse directed adjacency structure keyed
by `CxId` that stores edge weights (agreement × directional confidence) and per-node
frequency weights (A29: recurrence frequency raises in-degree/weight — recurring
constellations are strong kernel candidates). This is the shared data structure
consumed by traversal (T01), SCC (T03), betweenness (T04), and the graph builder (T05).

## Build (checklist of concrete, code-level steps)

- [ ] `pub struct AssocGraph`: fields — `nodes: Vec<NodeEntry>`, `edges: Vec<Edge>`,
  `adj: Vec<Range<usize>>` (CSR offsets), `id_to_idx: HashMap<CxId, usize>`.
- [ ] `pub struct NodeEntry { id: CxId, frequency_weight: f32 }` — frequency from
  recurrence counter (A29); default `1.0`; must be finite and > 0.
- [ ] `pub struct Edge { src: usize, dst: usize, weight: f32 }` — `weight` =
  agreement × directional_confidence; must be finite, 0.0 ≤ weight ≤ 1.0.
- [ ] `AssocGraph::builder() -> AssocGraphBuilder` — accumulate nodes/edges then
  `build()` → sorted CSR; deduplicates parallel edges by keeping max weight.
- [ ] `fn add_node(id: CxId, frequency_weight: f32)` on builder;
  duplicate CxId → error `CALYX_GRAPH_DUPLICATE_NODE`.
- [ ] `fn add_edge(src: CxId, dst: CxId, weight: f32)` on builder;
  unknown CxId → error `CALYX_GRAPH_UNKNOWN_NODE`;
  weight out of range → error `CALYX_GRAPH_INVALID_WEIGHT`.
- [ ] `fn out_neighbors(node: CxId) -> &[Edge]`; `fn in_degree(node: CxId) -> usize`;
  `fn node_weight(node: CxId) -> f32`.
- [ ] Memory layout: edge slice per node; total memory O(V + E).

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: build triangle `A→B→C→A` with weights `0.8, 0.6, 0.9`;
  `out_neighbors(A)` = `[(A→B, 0.8)]`; `in_degree(B)` = `1`.
- [ ] unit: frequency weight 3.0 on node A; `node_weight(A)` = `3.0` after build.
- [ ] proptest: for any acyclic random graph, total edge count equals sum of
  `out_degree(v)` over all v.
- [ ] edge: empty graph (`0` nodes) builds without panic; queries on absent CxId
  → `CALYX_GRAPH_UNKNOWN_NODE`.
- [ ] edge: parallel edges `A→B` with weights `0.3` and `0.7` →
  after build, single edge `A→B` with weight `0.7` (max).
- [ ] edge: self-loop `A→A` → accepted (self-loops are valid in the association
  graph; SCCs will absorb them).
- [ ] fail-closed: `add_edge` with weight `1.1` → `CALYX_GRAPH_INVALID_WEIGHT`;
  weight `-0.1` → same error.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `cargo test -p calyx-paths graph -- --nocapture` stdout on aiwonder.
- **Readback:** `cargo test -p calyx-paths graph 2>&1 | tee /tmp/ph31_t02_fsv.txt && cat /tmp/ph31_t02_fsv.txt`.
- **Prove:** all graph unit + proptest pass; printed edge list for the triangle test
  must show exactly three entries with the correct weights; frequency weight `3.0`
  printed for node A; no tests silently skipped.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH31 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
