# PH52 · T05 — Grounded label propagation (Laplacian heat diffusion)

| Field | Value |
|---|---|
| **Phase** | PH52 — Advanced math |
| **Stage** | S11 — Oracle & AGI Layer |
| **Crate** | `calyx-lodestar` |
| **Files** | `crates/calyx-lodestar/src/label_propagation.rs` (≤500) |
| **Depends on** | PH32 (MFVS kernel — the fixed/anchored nodes), PH31 (sparse graph — Laplacian matrix), PH13 (Forge CUDA — sparse linear algebra for the diffusion solve) |
| **Axioms** | A2, A16, A11 |
| **PRD** | `dbprdplans/26 §11.2` |

## Goal

Implement Laplacian heat diffusion from the grounded kernel anchors across the full
association graph, giving every non-anchored constellation a **propagated, provisional
grounding confidence** that decays with graph distance from a real anchor. This is the
operational realization of "1% grounds 99% by association" (`26 §11.2`): grounding flows
from the kernel's anchored nodes to the whole corpus along the edges. Propagated labels
are tagged `provisional` with a `confidence` the real-anchor frontier sharpens. Never
confused with a measured anchor (A2/A16). Never replaces the grounded MFVS kernel —
extends it to un-anchored nodes (`26 §11.2`).

## Build (checklist of concrete, code-level steps)

- [ ] `pub fn propagate_labels(graph: &SparseGraph, kernel_labels: &[(NodeId, f32)], max_iter: usize, tol: f32) -> Result<Vec<PropagatedLabel>, PropagationError>` — solves the Laplacian system holding kernel (anchored) nodes fixed, letting the rest settle via iterative solve (Gauss-Seidel or power iteration); `kernel_labels` = `(node_id, grounding_confidence)` for each MFVS kernel node
- [ ] `struct PropagatedLabel { node_id: NodeId, label: f32, confidence: f32, hop_distance: u32, provisional: bool }` — `provisional = true` for all non-kernel nodes (A2/A16); `provisional = false` only for kernel nodes with measured grounding; `confidence` decays with `hop_distance`
- [ ] **Heat diffusion iteration:** `L · f = 0` with Dirichlet boundary at kernel nodes (held fixed); iterative: `f_t+1[non_kernel] = (D^{-1} A) · f_t`; stop when `max(|f_{t+1} - f_t|) < tol`
- [ ] **Confidence decay:** `confidence[v] = kernel_confidence * exp(−λ · hop_distance[v])` where λ is Anneal-tunable (default `0.5`); `hop_distance` = shortest-path distance from any kernel node (BFS on undirected graph)
- [ ] `provisional = true` for all propagated (non-kernel) nodes; never set `provisional = false` on a propagated node even at confidence = 1.0 — the flag is epistemic, not quantitative
- [ ] Disconnected component: nodes with no path to any kernel node receive `confidence = 0.0`, `provisional = true`, `hop_distance = u32::MAX`; no error — this is valid (A16 fail-closed via `provisional` flag)
- [ ] Write a `LedgerRef` entry recording the propagation run: `(graph_version, kernel_hash, n_propagated)` (A15)
- [ ] `struct PropagationError` with variants: `GraphEmpty`, `NoKernelNodes`, `NotConverged { iter: usize }` — each with `CALYX_PROP_*` code

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: 5-node path graph `0—1—2—3—4`; kernel anchor at node 0 (confidence = 1.0); propagation gives node 1 confidence > node 2 > node 3 > node 4 (monotonically decaying)
- [ ] unit: planted rare-class carrier: star graph with hub=kernel (confidence=1.0); all 5 leaves receive equal confidence ≈ `exp(−0.5)` ± 0.01
- [ ] unit: disconnected node receives `confidence = 0.0`, `provisional = true`, `hop_distance = u32::MAX`
- [ ] unit: kernel node itself has `provisional = false`, `confidence = kernel_confidence` (unchanged)
- [ ] proptest: for any connected graph, `confidence[v]` is monotonically non-increasing with `hop_distance[v]` from the nearest kernel node
- [ ] edge (≥3): no kernel nodes → `PropagationError::NoKernelNodes`; single-node graph + kernel = that node → returns one `PropagatedLabel` with `provisional = false`; cycle graph with one kernel node → all nodes reachable, confidence decays by path length
- [ ] fail-closed: `NotConverged` after `max_iter` → `Err(PropagationError::NotConverged)`; no partial result silently returned

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `Vec<PropagatedLabel>` JSON from `calyx readback label_propagation --scope <scope_id>`; Ledger CF entry for the propagation run
- **Readback:**
  ```
  cargo test -p calyx-lodestar -- label_propagation --nocapture 2>&1 | tee /tmp/ph52_prop.log
  grep "hop_distance\|confidence\|provisional" /tmp/ph52_prop.log
  # Path graph: confidence decreasing; kernel node: provisional=false
  # Disconnected: confidence=0, provisional=true
  ```
- **Prove:** path-graph test shows monotonically decreasing confidence from kernel outward; kernel node shows `provisional = false`; disconnected node shows `confidence = 0.0` and `provisional = true`; Ledger entry written (xxd confirms)

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH52 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
