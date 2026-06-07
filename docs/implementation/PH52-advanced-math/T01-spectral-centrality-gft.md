# PH52 · T01 — Spectral centrality + GFT (Lanczos + Forge eigensolve)

| Field | Value |
|---|---|
| **Phase** | PH52 — Advanced math |
| **Stage** | S11 — Oracle & AGI Layer |
| **Crate** | `calyx-mincut` |
| **Files** | `crates/calyx-mincut/src/spectral.rs` (≤500) |
| **Depends on** | PH31 (sparse graph — adjacency matrix input), PH13 (Forge CUDA — matrix-vector products for Lanczos), PH46 (Anneal — which eigenvectors to cache) |
| **Axioms** | A30, A2 |
| **PRD** | `dbprdplans/26 §2` |

## Goal

Implement spectral analysis on the association graph: (1) **eigenvector centrality** via
the power method / Lanczos (the principal eigenvector of the adjacency operator) — a
continuous kernel-importance ranking complementing the discrete MFVS kernel; (2) **Graph
Fourier Transform (GFT)** — project a signal-over-constellations onto Laplacian
eigenvectors for smooth/sharp decomposition and denoising; (3) **spectral gap** —
identify structural bottlenecks. All complement but never replace the grounded MFVS
kernel (`26 §2`; `A2` boundary documented in code). Results cached per
`(scope, panel_version)` and Anneal-refreshed.

## Build (checklist of concrete, code-level steps)

- [ ] `pub fn eigenvector_centrality(graph: &SparseGraph, max_iter: usize, tol: f32) -> Result<Vec<(NodeId, f32)>, SpectralError>` — power iteration on the adjacency matrix; uses Forge batched matrix-vector multiply; converge when `||x_{t+1} - x_t|| < tol`; return sorted descending by centrality score
- [ ] `pub fn laplacian_eigenmaps(graph: &SparseGraph, k: usize) -> Result<Vec<EigenPair>, SpectralError>` — Lanczos iteration for the `k` smallest non-zero Laplacian eigenvalues + eigenvectors; re-orthogonalize every 10 steps; returns `Vec<EigenPair { eigenvalue: f32, eigenvector: Vec<f32> }>`
- [ ] `pub fn gft_project(signal: &[f32], eigenvectors: &[EigenPair]) -> Vec<f32>` — project a signal (one value per constellation) onto the Laplacian eigenvectors: `ĝ_k = <g, u_k>`; returns the frequency-domain representation
- [ ] `pub fn gft_reconstruct(coefficients: &[f32], eigenvectors: &[EigenPair]) -> Vec<f32>` — inverse GFT: `g = Σ_k ĝ_k · u_k`; low-pass filter = zero out high-frequency (large eigenvalue) coefficients before reconstruction
- [ ] `pub fn spectral_gap(eigenmaps: &[EigenPair]) -> f32` — gap between eigenvalue 1 and eigenvalue 2; large gap = well-separated clusters; small gap = cohesive
- [ ] **Honesty assert:** `// IMPORTANT: spectral centrality is structure-only; the MFVS kernel is outcome-anchored (A2). Centrality proposes candidates; grounding (oracle anchor) confirms them.` — as a code comment in `spectral.rs`
- [ ] Cache key `(scope, panel_version)` stored in a `SpectralCache`; invalidated on Anneal trigger
- [ ] `struct SpectralError` with variants: `NotConverged { iterations: usize }`, `GraphTooSmall { n: usize, required: usize }`, `SingularMatrix` — each with `CALYX_SPECTRAL_*` code

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: 4-node cycle graph `0→1→2→3→0` (uniform weights) → `eigenvector_centrality` returns all four nodes with equal score ± 1e-3
- [ ] unit: planted two-community graph (two cliques of 5, one bridge edge) → `spectral_gap` detects the bottleneck; second eigenvector bisects the two communities (positive on one side, negative on the other)
- [ ] unit: GFT round-trip: `gft_reconstruct(gft_project(signal, eigs), eigs) ≈ signal ± 1e-3`
- [ ] unit: low-pass filter via GFT removes a planted high-frequency signal (checkerboard) while preserving a planted low-frequency signal (smooth gradient) ± 1e-2
- [ ] proptest: `eigenvector_centrality` scores all ∈ `[0.0, 1.0]`; max score = 1.0 (normalized); sum > 0
- [ ] edge (≥3): 1-node graph → `GraphTooSmall` error; disconnected graph → `spectral_gap = 0` (fiedler eigenvalue = 0); star graph → hub has highest centrality
- [ ] fail-closed: Lanczos not converged within `max_iter` → `SpectralError::NotConverged`; no silent return of unconverged eigenvectors

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `calyx readback spectral_centrality --scope <scope_id>`; Lanczos eigenvalue output; planted-community graph test
- **Readback:**
  ```
  cargo test -p calyx-mincut -- spectral --nocapture 2>&1 | tee /tmp/ph52_spectral.log
  grep "community_bisection" /tmp/ph52_spectral.log  # second eigenvector sign pattern
  grep "spectral_gap" /tmp/ph52_spectral.log         # > 0 for two-community graph
  grep "gft_roundtrip_error" /tmp/ph52_spectral.log  # < 1e-3
  ```
- **Prove:** planted two-community graph: second Laplacian eigenvector has opposite signs on the two clusters (read the eigenvector values in the log); GFT round-trip error ≤ 1e-3; centrality hub node has score ≥ 2× bridge node

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] [Forge-touching] CPU↔GPU bit-parity ≤ 1e-3 on the golden set
- [ ] FSV evidence (readback output / screenshot) attached to the PH52 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
