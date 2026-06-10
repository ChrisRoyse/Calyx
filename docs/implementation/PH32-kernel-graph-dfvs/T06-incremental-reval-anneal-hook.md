# PH32 - T06 - Incremental re-eval hook for Anneal

> **STATUS: DONE / FSV-signed-off.** Implemented in
> `crates/calyx-lodestar/src/incremental.rs` with edge-weight dirty marking,
> leaf-add handling, SCC-merge full-rebuild detection, kernel-member removal
> signaling, and conservative dirty rebuild. #346 corrected the overclaim:
> `rebuild_dirty()` reruns the full kernel pipeline when dirty or stale; true
> dirty-SCC-only recompute remains deferred to Anneal/PH43. aiwonder FSV
> readback: `ph32-incremental-readback.json`.

| Field | Value |
|---|---|
| **Phase** | PH32 - Kernel-graph (~10% target) + directed MFVS (~1% target) |
| **Stage** | S6 - Lodestar Kernel |
| **Crate** | `calyx-lodestar` |
| **Files** | `crates/calyx-lodestar/src/incremental.rs` (<=500) |
| **Depends on** | T05 (`Kernel`, `build_kernel_pipeline`) |
| **Axioms** | A10, A14 |
| **PRD** | `dbprdplans/08 section 3` incremental re-eval |

## Goal

Implement `IncrementalKernelEval`: a delta-update structure that accepts
new/removed/reweighted edges from Anneal and determines whether the current
`Kernel` is still valid or needs recomputation. In the current PH32
implementation, edge-weight changes and leaf-node additions mark dirty SCCs for
diagnostics, but `rebuild_dirty()` deliberately reruns the full
`build_kernel_pipeline` as a conservative fail-closed fallback. Full topology
changes such as SCC splits/merges and node removals mark the evaluator stale and
require a full rebuild. True dirty-SCC-only recompute is future PH43/Anneal work.

## Build

- [x] `pub struct IncrementalKernelEval { kernel, graph, anchors, dirty_sccs, params, stale }`.
- [x] `apply_edge_weight_change(src, dst, new_weight)`: updates edge weight in
  `graph`, marks the SCCs of `src` and `dst` as dirty, and returns
  `IncrementalResult::Dirty { affected_sccs }`.
- [x] `apply_node_add(id, frequency, edges)`: adds a leaf node; if the node is a
  cycle-closer, returns `IncrementalResult::FullRebuildRequired`.
- [x] `apply_node_remove(id)`: removes a node; if it was in `kernel.members`,
  returns `IncrementalResult::KernelMemberRemoved { id }`; otherwise returns
  `FullRebuildRequired` because removal can split or reindex SCCs.
- [x] `rebuild_dirty()`: conservatively reruns the full SCC + betweenness + DFVS
  pipeline when dirty or stale, then clears `dirty_sccs` and `stale`.
- [x] `IncrementalResult` is `#[must_use]`; callers must handle all variants.

## Tests

- [x] unit: triangle kernel; `apply_edge_weight_change(A, B, 0.1)` returns
  `Dirty`, and `rebuild_dirty` keeps the kernel valid.
- [x] unit: add leaf `D` with single edge `D -> A`; after `rebuild_dirty`, `D`
  is not in `kernel.members`.
- [x] unit: remove a non-kernel node after a leaf add; returns
  `FullRebuildRequired`, and `rebuild_dirty` clears `stale` instead of no-oping.
- [x] unit: add node `E` with edges `E -> A` and `B -> E`; returns
  `FullRebuildRequired` and marks the kernel stale.
- [x] unit: remove a node in `kernel.members`; returns
  `KernelMemberRemoved { id }`.
- [x] edge: unknown node on `apply_edge_weight_change` returns the graph error
  instead of mutating the kernel.
- [x] edge: `rebuild_dirty` with no dirty SCCs and not stale is a no-op.
- [x] fail-closed: invalid add-node weights are rejected by the graph builder
  before the kernel is mutated.

## FSV

- **SoT:** `ph32-incremental-readback.json` and the aiwonder test log.
- **Readback:** `cargo test -p calyx-lodestar incremental -- --nocapture`
  writes the readback JSON when `CALYX_FSV_ROOT` is set, then the agent reads
  the JSON bytes back from aiwonder.
- **Prove:** leaf-add readback confirms the leaf is excluded from
  `kernel.members`; non-kernel node removal prints `FullRebuildRequired` and
  clears `stale` after rebuild; cycle-closer prints `FullRebuildRequired`;
  kernel-member removal prints `KernelMemberRemoved`.

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder for the
  issue scope.
- [x] file(s) <= 500 lines.
- [x] FSV evidence attached to the PH32 GitHub issue.
- [x] no anti-pattern: no flattening, no trusted claim without grounding, and no
  harness-as-FSV.
