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

## Current state

✅ **DONE / FSV-signed-off on aiwonder.** `calyx-paths` now owns `AssocGraph`,
hop attenuation, and bounded reach/reach_scored traversal. `calyx-mincut` now
owns Tarjan SCC condensation, directed Brandes betweenness, Loom-style
agreement/citation graph building, and serializable LP scaffold types for PH32.
The graph-builder core is proven from deterministic CxId edge inputs. The real
Loom xterm/agreement CF adapter into those inputs lives in
`crates/calyx-lodestar/src/loom_assoc.rs` and is FSV-backed by #293.

FSV root: `/home/croyse/calyx/data/fsv-ph31-20260608`.

The ContextGraph project remains an allowed seed source per `19 §6`, but PH31
landed as Calyx-native Rust over `CxId` and `AssocGraph`; it does not link or
import the live ContextGraph project.

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
| T01 | Seed + adapt calyx-paths traversal + hop-attenuation | — | ✅ FSV |
| T02 | Sparse AssocGraph with frequency-weighted nodes | T01 | ✅ FSV |
| T03 | Tarjan SCC condensation | T02 | ✅ FSV |
| T04 | Brandes betweenness centrality | T03 | ✅ FSV |
| T05 | Association graph builder from Loom agreements | T02 | ✅ FSV |
| T06 | LP scaffolding data types | T03 | ✅ FSV |

## FSV exit gate (the phase is DONE only when this is byte-proven on aiwonder)

On a **planted graph** (known SCCs, known betweenness scores):
1. `tarjan_scc(planted_graph)` → SCC partition matches the planted partition
   exactly; read the computed SCC members from a `calyx readback` or debug dump.
2. Brandes betweenness scores on the same planted graph match a reference
   implementation within ε = 1e-6 (normalized); read both vectors and diff.
3. Evidence (stdout + comparison table) attached to the PH31 GitHub issue.

Readback hashes:

| File | SHA-256 |
|---|---|
| `ph31-paths-graph-readback.json` | `50f76709717229941761aea13b2c1da8fa24303b9e7ca22173c376dbe913a6e6` |
| `ph31-paths-traversal-readback.json` | `0f6aff06df14afe10ecb8b8b4e5aa6262b097a2c603e08289677051fffdc48d1` |
| `ph31-scc-readback.json` | `328252fc7dd35aea7bd34f01fabbaf396f81266c1b9d9dfcb43a979ce7e5998a` |
| `ph31-betweenness-readback.json` | `fd8d05ce36f723bf27dfbcb3c9901a0faf69f08d44883daf71d2d97aacd7b17d` |
| `ph31-graph-builder-readback.json` | `13b97defa1e59d700c163a66ea3c5037a0bf083381cb545814b201e17eaa8313` |
| `ph31-lp-readback.json` | `cf4d27c4de5d8e0c5f8ef6790afa0bf7d031ad30bdb3c2427f3c33183906e5d1` |

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
