# PH13 · T05 — GPU bitonic topk

| Field | Value |
|---|---|
| **Phase** | PH13 — CUDA sm_120 Backend + Bit-Parity |
| **Stage** | S2 — Forge Math Runtime |
| **Crate** | `calyx-forge` |
| **Files** | `crates/calyx-forge/src/cuda/topk.rs` (≤500), `crates/calyx-forge/src/cuda/kernels/topk.cu` (≤500) |
| **Depends on** | T01, T02 (this phase) |
| **Axioms** | A13, A16 |
| **PRD** | `dbprdplans/13 §3` |

## Goal

Implement a GPU bitonic sort topk that is deterministic (tie-break by lower index
wins, matching PH12 T04's CPU topk contract) and returns the same ranked indices
as the CPU path within ≤ 1e-3 score rel for the golden set. This kernel is on the
ANN rerank hot path. Current CUDA top-k is exact for global
`k <= CUDA_EXACT_TOPK_MAX_K` (`1024`); larger `k` fails loud with
`CALYX_FORGE_SHAPE_MISMATCH` until a multi-pass exact CUDA merge is implemented.

## Build (checklist of concrete, code-level steps)

- [x] `topk.cu` `bitonic_topk_f32` kernel: in-shared-memory bitonic sort; input
  is `scores[n]`; output is top-k `(index, score)` pairs sorted descending;
  tie-break: when scores equal, lower index retained; block handles up to 1024
  elements in shared memory; for larger n, iterative passes
- [x] `// DETERMINISM:` comment on every compare-swap: `// DETERMINISM: ties broken
  by index (lower index wins); no warp-divergent paths on index comparison`
- [x] NaN in input → kernel writes sentinel `(-1, -2.0f)` for that slot; host
  detects sentinel and returns `ForgeError::NumericalInvariant`
- [x] `src/cuda/topk.rs`: `pub fn topk_gpu(ctx: &CudaContext, scores: &CudaSlice<f32>, k: usize, n: usize) -> Result<Vec<(usize, f32)>, ForgeError>`
  — htod copy, kernel dispatch, dtoh copy, sentinel check, return sorted vec
- [x] `impl Backend for CudaBackend`: `topk` delegates to `topk_gpu`
- [x] For k > n: return all n scores sorted (same contract as CPU topk)
- [x] For k == 0: return empty vec without kernel launch
- [x] For global k > `CUDA_EXACT_TOPK_MAX_K` (`1024`): return
      `CALYX_FORGE_SHAPE_MISMATCH` instead of
      merging non-exact per-chunk winners

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: `topk_gpu` on `[0.1, 0.9, 0.5, 0.9]`, k=2 → `[(1, 0.9), (3, 0.9)]`
  (same as CPU; lower-index tie-break)
- [x] unit: k ≥ n → returns all 4 elements sorted descending
- [x] proptest: GPU topk indices match CPU topk indices for random score arrays
  length 16–512, k=8, seed=42
- [x] proptest: GPU topk result is sorted descending by score (within 1e-5)
- [x] edge (≥3): (1) all equal scores → indices 0,1,…,k-1; (2) n=1 k=1; (3) k=0 → empty
- [x] fail-closed: NaN in scores → `CALYX_FORGE_NUMERICAL_INVARIANT`

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `tests/cuda_parity.rs::topk_tie_break_gpu_matches_cpu` on aiwonder
- **Readback:**
  ```bash
  source $CALYX_HOME/repo/env.sh
  cargo test -p calyx-forge --features cuda cuda::topk -- --nocapture 2>&1 \
    | grep -E "PASSED|FAILED|index|tie"
  ```
- **Prove:** tie-break test PASSED showing `[(1, 0.9), (3, 0.9)]`; proptest PASSED;
  `cuda-topk-success-readback.json` shows sorted bytes; large-k fail-loud
  readback prints `CALYX_FORGE_SHAPE_MISMATCH` and
  `cuda_exact_topk_max_k=1024`; absent: any reversed tie-break, any panic, any
  silent non-exact large-k merge

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] CPU↔GPU bit-parity ≤ 1e-3 on the golden set (enforced in T06)
- [x] FSV evidence attached to PH13 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
