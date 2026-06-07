# PH31 — mincut/paths: graph build + SCC + betweenness

**Stage:** S6 — Lodestar Kernel  ·  **Crate:** `calyx-mincut`, `calyx-paths`  ·
**PRD roadmap:** P5  ·  **Axioms:** A29, `19 §6`

## Objective

Build the directed association graph over constellations and implement the
graph-primitive layer that the MFVS pipeline (PH32) requires: Tarjan SCC
condensation, betweenness centrality, hop-attenuated traversal, and LP scaffolding.
This phase seeds both `calyx-paths` and `calyx-mincut` from the ContextGraph
`mincut`/`paths`/`solver` sources (copied into CALYX_HOME, never linked),
then adapts them to Calyx's `CxId`-keyed sparse adjacency and the agreement ×
directional-confidence edge model from PH27.

## Dependencies

- **Phases:** PH27 (agreement graph — edge source), PH09 (CxId, Anchor,
  constellation CRUD — node source)
- **Provides for:** PH32 (kernel-graph + MFVS uses SCC condensate, betweenness,
  LP scaffolding), PH33 (kernel index + answer traversal via hop-attenuation),
  PH34 (multi-scope build_kernel uses the same graph layer)

## Current state (build off what exists)

Both `calyx-paths` and `calyx-mincut` are 9-line stubs (greenfield). The
ContextGraph project ships `context-graph-mincut`, `context-graph-paths`, and
`context-graph-solver` crates; per `19 §6` / DOCTRINE reuse rule, their source is
**copied** into `crates/calyx-paths/src/` and `crates/calyx-mincut/src/` as seeds,
then adapted. Never link or import the live ContextGraph project.

## Deliverables (file plan, each ≤500 lines)

| File | Responsibility |
|---|---|
| `crates/calyx-paths/src/lib.rs` | crate root; re-exports `graph`, `traversal`, `attenuation` |
| `crates/calyx-paths/src/graph.rs` | `AssocGraph`: sparse adjacency (CSR-style), `CxId`-keyed; edge = `(src, dst, weight: f32)`; frequency→node weight (A29) |
| `crates/calyx-paths/src/traversal.rs` | bidirectional BFS/DFS; `reach(src, dst, max_hops)` → `Vec<CxId>`; hop-attenuation `0.9^hop` applied to each path score |
| `crates/calyx-paths/src/attenuation.rs` | `attenuate(base_score, hops) -> f32` = `base_score * 0.9_f32.powi(hops)`; inverse for re-ranking |
| `crates/calyx-mincut/src/lib.rs` | crate root; re-exports `scc`, `betweenness`, `lp_scaffold` |
| `crates/calyx-mincut/src/scc.rs` | Tarjan SCC condensation; `tarjan_scc(graph) -> Vec<Vec<CxId>>`; condensate DAG |
| `crates/calyx-mincut/src/betweenness.rs` | Brandes betweenness centrality; `betweenness(graph) -> HashMap<CxId, f64>`; normalized; sparse shortcut for scale-free |
| `crates/calyx-mincut/src/lp_scaffold.rs` | LP variable/constraint scaffolding for MFVS (used by PH32); data types only, no solver yet |
| `crates/calyx-mincut/src/graph_builder.rs` | `build_assoc_graph(loom_agreements, anchors) -> AssocGraph`; edge weight = agreement × directional_confidence; citation/entity edges; frequency raises node weight |

## Tasks (atomic — all must pass for the phase to be DONE)

| Card | Title | Depends |
|---|---|---|
| T01 | Seed + adapt calyx-paths traversal + hop-attenuation | — |
| T02 | Sparse AssocGraph with frequency-weighted nodes | T01 |
| T03 | Tarjan SCC condensation | T02 |
| T04 | Brandes betweenness centrality | T03 |
| T05 | Association graph builder from Loom agreements | T02 |
| T06 | LP scaffolding data types | T03 |

## FSV exit gate (the phase is DONE only when this is byte-proven on aiwonder)

On a **planted graph** (known SCCs, known betweenness scores):
1. `tarjan_scc(planted_graph)` → SCC partition matches the planted partition
   exactly; read the computed SCC members from a `calyx readback` or debug dump.
2. Brandes betweenness scores on the same planted graph match a reference
   implementation within ε = 1e-6 (normalized); read both vectors and diff.
3. Evidence (stdout + comparison table) attached to the PH31 GitHub issue.

## Risks / landmines

- **ContextGraph source copyright / API drift:** copy verbatim then rename
  types; track which commits were seeded from so diffs stay auditable.
- **Scale-free betweenness:** Brandes is `O(VE)` — fine for the kernel-graph
  (~10% of corpus); do not run on the full billion-node graph without the
  SCC-condense + kernel-graph filter first.
- **f32 vs f64 edge weights:** agreement scores are f32; betweenness accumulates
  f64 intermediates to avoid catastrophic cancellation — keep types distinct.
- **Frequency raises in-degree (A29):** weight must feed in-degree, not create
  new edges; test that a high-frequency node increases its own weight but does
  not fabricate adjacency.
