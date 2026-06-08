# PH32 · T06 — Incremental re-eval hook for Anneal

> **STATUS: ✅ DONE / FSV-signed-off.** Implemented in
> `crates/calyx-lodestar/src/incremental.rs` with edge-weight dirty marking,
> leaf-add handling, SCC-merge full-rebuild detection, kernel-member removal
> signaling, and dirty rebuild. aiwonder FSV readback:
> `ph32-incremental-readback.json`.

| Field | Value |
|---|---|
| **Phase** | PH32 — Kernel-graph (~10%) + directed MFVS (~1%) |
| **Stage** | S6 — Lodestar Kernel |
| **Crate** | `calyx-lodestar` |
| **Files** | `crates/calyx-lodestar/src/incremental.rs` (≤500) |
| **Depends on** | T05 (`Kernel`, `build_kernel_pipeline`) |
| **Axioms** | A10, A14 |
| **PRD** | `dbprdplans/08 §3` ("Incremental: as constellations arrive, Anneal re-evaluates the kernel, not recomputed from scratch") |

## Goal

Implement `IncrementalKernelEval`: a delta-update structure that accepts
new/removed/reweighted edges from Anneal and determines whether the current
`Kernel` is still valid or needs partial recomputation. For this phase, only
edge-weight changes and leaf-node additions are handled incrementally; full
topology changes (SCC splits/merges) trigger a full rebuild with a warning.
This keeps the kernel current without paying the full `build_kernel_pipeline`
cost on each corpus update.

## Build (checklist of concrete, code-level steps)

- [ ] `pub struct IncrementalKernelEval { kernel: Kernel, graph: AssocGraph, dirty_sccs: HashSet<usize>, params: KernelParams }`.
- [ ] `pub fn apply_edge_weight_change(&mut self, src: CxId, dst: CxId, new_weight: f32) -> IncrementalResult` — updates edge weight in `graph`; marks the SCCs of `src` and `dst` as dirty; returns `IncrementalResult::Dirty { affected_sccs }`.
- [ ] `pub fn apply_node_add(&mut self, id: CxId, frequency: f32, edges: Vec<(CxId, f32)>) -> IncrementalResult` — adds a leaf node (no inbound edges from existing nodes); if the node is a cycle-closer (creates a new SCC) → `IncrementalResult::FullRebuildRequired`.
- [ ] `pub fn apply_node_remove(&mut self, id: CxId) -> IncrementalResult` — removes a node; if it was in `kernel.members` → `IncrementalResult::KernelMemberRemoved { id }`.
- [ ] `pub fn rebuild_dirty(&mut self) -> Result<(), CalyxError>` — re-runs SCC + betweenness + DFVS only on the dirty subgraph; merges result back into `kernel.members`; clears `dirty_sccs`.
- [ ] `IncrementalResult` is `#[must_use]`; callers must handle all variants.
- [ ] Clock injection: `apply_*` methods accept `clock: &dyn Clock` for the mutation
  timestamp; no `SystemTime::now()` in logic (DOCTRINE).

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: triangle kernel; `apply_edge_weight_change(A, B, 0.1)` → `dirty_sccs = {0}`;
  `rebuild_dirty` re-runs and kernel stays valid (FVS still present).
- [ ] unit: add leaf `D` with single edge `D→A` (no cycle) → `IncrementalResult::Dirty`;
  after `rebuild_dirty`, `D` is NOT in kernel.members (leaf, not a cycle node).
- [ ] unit: add node `E` with edges `E→A` AND `B→E` (creates new cycle) →
  `IncrementalResult::FullRebuildRequired`; kernel marked stale.
- [ ] unit: remove a node that was in `kernel.members` → `IncrementalResult::KernelMemberRemoved`
  with the correct `CxId`.
- [ ] edge: `apply_edge_weight_change` to a node not in graph → `CALYX_GRAPH_UNKNOWN_NODE`.
- [ ] edge: `rebuild_dirty` with `dirty_sccs = {}` (nothing dirty) → no-op; kernel unchanged.
- [ ] fail-closed: `apply_node_add` with `frequency < 1.0` → `CALYX_GRAPH_INVALID_WEIGHT`;
  kernel not modified.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `cargo test -p calyx-lodestar incremental -- --nocapture` stdout.
- **Readback:** `cargo test -p calyx-lodestar incremental 2>&1 | tee /tmp/ph32_t06_fsv.txt && cat /tmp/ph32_t06_fsv.txt`.
- **Prove:** leaf-add test prints `dirty_sccs` set and confirms leaf not in
  kernel.members after rebuild; cycle-closer test prints `FullRebuildRequired`;
  all tests pass; output attached to PH32 GitHub issue.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH32 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
