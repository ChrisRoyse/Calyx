# PH32 · T01 — Kernel-graph selection: degree + betweenness + groundedness filter

> **STATUS: ✅ DONE / FSV-signed-off.** Implemented in
> `crates/calyx-lodestar/src/kernel_graph.rs` with degree, betweenness, and
> groundedness-distance scoring plus deterministic top-fraction selection.
> aiwonder FSV readback: `ph32-kernel-graph-readback.json`.

| Field | Value |
|---|---|
| **Phase** | PH32 — Kernel-graph (~10%) + directed MFVS (~1%) |
| **Stage** | S6 — Lodestar Kernel |
| **Crate** | `calyx-lodestar` |
| **Files** | `crates/calyx-lodestar/src/lib.rs` (≤500), `crates/calyx-lodestar/src/kernel_graph.rs` (≤500) |
| **Depends on** | PH31 (T02 `AssocGraph`, T03 `SccResult`, T04 `betweenness`) |
| **Axioms** | A10, A29 |
| **PRD** | `dbprdplans/08 §3` (Stage 2: Kernel-graph ~10%) |

## Goal

Implement Stage 2 of the MFVS pipeline: from the condensed `AssocGraph` (post
Tarjan SCC from PH31), select the ~10% "kernel-graph" consisting of nodes with
high in-degree + out-degree, high betweenness centrality, and low groundedness-
distance (closeness to an `Anchor`). This filter dramatically reduces the graph
before the expensive MFVS step. Result: a `KernelGraph` sub-graph carrying the
selected nodes and their interconnecting edges.

## Build (checklist of concrete, code-level steps)

- [ ] `pub struct KernelGraphParams { degree_percentile: f32, betweenness_percentile: f32, max_groundedness_distance: usize, target_fraction: f32 }` — defaults: `degree_percentile=0.80`, `betweenness_percentile=0.80`, `max_groundedness_distance=3`, `target_fraction=0.10`.
- [ ] `pub struct KernelGraph { graph: AssocGraph, source_fraction: f32, params: KernelGraphParams }`.
- [ ] `pub fn select_kernel_graph(graph: &AssocGraph, scc: &SccResult, betweenness: &HashMap<CxId, f64>, anchors: &[CxId], params: &KernelGraphParams) -> Result<KernelGraph, CalyxError>`.
- [ ] Score each condensed SCC node: `score = w_deg*(in_deg+out_deg)/max_deg + w_bet*betweenness + w_gnd*(1.0 - gnd_dist/max_gnd_dist)`; weights sum to 1.0.
- [ ] `groundedness_distance(node, anchors)` = BFS hop-count from `node` to nearest anchor in `graph`; uncreachable → `usize::MAX` (penalized in score).
- [ ] Select top-`(target_fraction * n)` nodes by score; include all edges between selected nodes.
- [ ] Log `actual_fraction = selected / total` (may differ from `target_fraction`).
- [ ] Empty graph (0 nodes) → `CALYX_KERNEL_EMPTY_GRAPH`.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: 10-node graph with 2 hubs (high degree) and 8 leaves; `target_fraction=0.20`
  → 2 nodes selected; both are the hubs; `source_fraction ≈ 0.20`.
- [ ] unit: anchor node `A` at distance 0 from itself; node `Z` with no path to any
  anchor → `Z`'s score is penalized by `max_gnd_dist`; `A` has higher score.
- [ ] unit: all nodes equally scoring → selection is deterministic (by `CxId` sort).
- [ ] proptest: `selected_count <= ceil(target_fraction * n) + 1` for any valid graph.
- [ ] edge: graph with 1 node, `target_fraction=0.10` → 1 node selected (ceil rounding).
- [ ] edge: `anchors = []` (no anchors) → all nodes have `gnd_dist = usize::MAX`;
  selection falls back to degree + betweenness only; no error.
- [ ] fail-closed: `target_fraction > 1.0` → `CALYX_KERNEL_INVALID_PARAMS`;
  `target_fraction <= 0.0` → same.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `cargo test -p calyx-lodestar kernel_graph -- --nocapture` stdout.
- **Readback:** `cargo test -p calyx-lodestar kernel_graph 2>&1 | tee /tmp/ph32_t01_fsv.txt && cat /tmp/ph32_t01_fsv.txt`.
- **Prove:** hub-selection test prints the two hub `CxId`s as selected; `source_fraction`
  printed ≈ 0.20; all tests pass; output attached to PH32 GitHub issue.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH32 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
