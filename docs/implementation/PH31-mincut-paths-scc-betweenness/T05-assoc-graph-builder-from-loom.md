# PH31 · T05 — Association graph builder from Loom agreements

> **STATUS: ✅ DONE / FSV-signed-off for the deterministic graph-builder core.**
> Implemented in
> `crates/calyx-mincut/src/graph_builder.rs` with agreement × directional
> confidence weights, recurrence frequency node weights, citation edge
> max-merge at `1.0`, and graph-weight fail-closed validation. aiwonder FSV
> readback: `ph31-graph-builder-readback.json`. The real Loom xterm CF adapter
> into Lodestar/Mincut is implemented in
> `crates/calyx-lodestar/src/loom_assoc.rs` for #293.

| Field | Value |
|---|---|
| **Phase** | PH31 — mincut/paths: graph build + SCC + betweenness |
| **Stage** | S6 — Lodestar Kernel |
| **Crate** | `calyx-mincut` |
| **Files** | `crates/calyx-mincut/src/graph_builder.rs` (≤500) |
| **Depends on** | T02 (`AssocGraph`, `AssocGraphBuilder`), T03 (`SccResult`) |
| **Axioms** | A29 |
| **PRD** | `dbprdplans/08 §2` |

## Goal

Implement `build_assoc_graph`: the entry-point that reads Loom agreement scores +
directional confidence values from PH27, plus citation/entity edges from the
constellation store (PH09 anchors), and produces a complete `AssocGraph` for the
MFVS pipeline. Recurrence frequency (A29) is applied as node weight; the resulting
graph has edge weight = agreement × directional_confidence.

## Build (checklist of concrete, code-level steps)

- [x] Define deterministic input types for the graph-builder core:
  `AgreementEdge { src: CxId, dst: CxId, agreement: f32, directional_confidence: f32 }`,
  `FrequencyEntry { cx_id: CxId, frequency: f32 }`,
  `CitationEdge { src: CxId, dst: CxId }`.
- [x] Wire real Loom xterm/agreement CF output into these CxId edge inputs (#293).
- [x] `pub fn build_assoc_graph(agreements: &[AgreementEdge], frequencies: &[FrequencyEntry], citations: &[CitationEdge]) -> Result<AssocGraph, CalyxError>`.
- [x] For each `AgreementEdge`: `edge_weight = agreement * directional_confidence`;
  both values must be in `[0.0, 1.0]` or return `CALYX_GRAPH_INVALID_WEIGHT`.
- [x] For each `FrequencyEntry`: node weight = `frequency` (≥ 1.0 required;
  < 1.0 → `CALYX_GRAPH_INVALID_WEIGHT`).
- [x] For each `CitationEdge`: add edge with weight `1.0` (explicit provenance
  link; always fully trusted); skip if node not present → `CALYX_GRAPH_UNKNOWN_NODE`.
- [x] Deduplication: parallel edges keep the max weight (citation edges can overlap
  with agreement edges — keep max).
- [x] Function is pure: same inputs → identical `AssocGraph` byte-for-byte (sorted
  CSR ensures determinism).

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: 3 agreements `A→B (0.8, 0.9)`, `B→C (0.6, 0.7)`, `C→A (1.0, 1.0)`;
  expected edge weights: `0.72`, `0.42`, `1.0`; verify with `out_neighbors`.
- [x] unit: frequency `[A=2.0, B=1.0]`; `node_weight(A)` = `2.0`, `node_weight(B)` = `1.0`.
- [x] unit: citation edge `A→C` alongside agreement `A→C (0.3, 0.5)`;
  final weight = `max(1.0, 0.15)` = `1.0`.
- [x] proptest: for `n` non-overlapping agreement edges, `edge_count(graph)` = `n`.
- [x] edge: empty input arrays → valid empty graph (0 nodes, 0 edges), no error.
- [x] edge: agreement with both `src` and `dst` equal (self-loop) → accepted,
  edge weight = `agreement * directional_confidence`.
- [x] fail-closed: `agreement = 1.1` → `CALYX_GRAPH_INVALID_WEIGHT`;
  `frequency = 0.5` (< 1.0) → `CALYX_GRAPH_INVALID_WEIGHT`.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** graph-builder core readback `ph31-graph-builder-readback.json`, plus
  #293 Loom adapter JSON readbacks under
  `/home/croyse/calyx/data/fsv-issue293-loom-assoc-graph-20260608`.
- **Readback:** run `cargo test -p calyx-lodestar loom_assoc -- --nocapture`
  with an explicit `CALYX_FSV_ROOT`, then separately `cat`
  `happy/loom-assoc-graph.json` and `edges/loom-assoc-errors.json`.
- **Prove:** unit test prints the three edge weights `0.72, 0.42, 1.0` for the
  triangle; citation-merge test prints final weight `1.0`; #293 readback shows
  persisted XTerm CF bytes, CxId edge provenance, and fail-closed missing mapping
  / missing directional-confidence codes.

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] FSV evidence (readback output / screenshot) attached to the PH31 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
