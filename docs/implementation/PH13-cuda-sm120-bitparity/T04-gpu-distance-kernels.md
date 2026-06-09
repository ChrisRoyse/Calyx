# PH13 · T04 — Fused GPU distance kernels (cosine / dot / l2)

| Field | Value |
|---|---|
| **Phase** | PH13 — CUDA sm_120 Backend + Bit-Parity |
| **Stage** | S2 — Forge Math Runtime |
| **Crate** | `calyx-forge` |
| **Files** | `crates/calyx-forge/src/cuda/distance.rs` (≤500), `crates/calyx-forge/src/cuda/kernels/distance.cu` (≤500) |
| **Depends on** | T01, T02 (this phase) |
| **Axioms** | A13, A16 |
| **PRD** | `dbprdplans/13 §3/§6`, `dbprdplans/23 §3` |

## Goal

Implement fused GPU `cosine_batch`, `dot_batch`, and `l2_batch` kernels for the
`CudaBackend`, dispatched via the PTX embedded in T02. Kernels compute in f32,
fuse normalize+dot in one pass (no separate kernel launch), and must agree with
the CPU distance kernels (PH12 T03) within ≤ 1e-3 rel on the golden set. NaN/Inf
detected on-device → `CALYX_FORGE_NUMERICAL_INVARIANT` returned.

## Build (checklist of concrete, code-level steps)

- [x] `distance.cu` `cosine_batch_f32` kernel: each thread handles one candidate;
  loads query and candidate vectors from global memory in coalesced 128-bit loads
  (float4); computes dot and norms in registers; warp-shuffle reduces to thread 0;
  writes cosine to `out[cand_idx]`; if norm is zero → writes `-2.0f` as sentinel
  (host-side checks for sentinel and returns `CALYX_FORGE_NUMERICAL_INVARIANT`)
- [x] `distance.cu` `dot_batch_f32` and `l2_batch_f32`: same structure, no norm division
- [x] `src/cuda/distance.rs`: `pub fn cosine_batch_gpu(ctx: &CudaContext, query: &CudaSlice<f32>, candidates: &CudaSlice<f32>, dim: usize, n_cands: usize, out: &mut CudaSlice<f32>) -> Result<(), ForgeError>`
  — load PTX via `ctx.inner.load_ptx(DISTANCE_PTX_BYTES, "distance", &["cosine_batch_f32"])`
  on first call (cached in `CudaContext`); launch with grid=(n_cands+255)/256, block=256
- [x] Sentinel check: after `dtoh_sync_copy`, scan `out` for values ≤ -1.5 → if found
  → `ForgeError::NumericalInvariant { op: "cosine_batch_gpu", detail: "zero-norm candidate at index {i}" }`
- [x] `impl Backend for CudaBackend`: `cosine`, `dot`, `l2` delegate to the GPU functions
  (copy host→device, dispatch, copy device→host)
- [x] `CALYX_FORGE_NUMERICAL_INVARIANT` returned (not logged-and-silenced) on any
  non-finite detected output (post-compute `check_finite` on the result buffer)

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: GPU cosine of orthogonal dim-128 vectors → `0.0` (within 1e-5)
- [x] unit: GPU cosine of identical unit vectors → `1.0` (within 1e-5)
- [x] unit: `l2_batch_gpu` with `q=[0,0]`, `c=[[3,4]]` → `25.0` (within 1e-4)
- [x] proptest: GPU cosine agrees with CPU cosine within 1e-3 rel for random dim-128
  vectors, 100 candidates, seed=42
- [x] edge (≥3): (1) `n_cands=1`; (2) `dim=1536`; (3) zero-norm candidate → `CALYX_FORGE_NUMERICAL_INVARIANT`
- [x] fail-closed: kernel launch on uninitialized context → `DeviceUnavailable`

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `tests/cuda_parity.rs::gpu_cosine_orthogonal` + `gpu_cosine_proptest` on aiwonder
- **Readback:**
  ```bash
  source $CALYX_HOME/repo/env.sh
  cargo test -p calyx-forge --features cuda cuda::distance -- --nocapture 2>&1 \
    | grep -E "PASSED|FAILED|cosine|rel_err"
  ```
- **Prove:** orthogonal and identical cosine tests PASSED; proptest PASSED with
  max rel_err ≤ 1e-3; absent: any sentinel `-2.0` leaking through to test output,
  any panic

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] CPU↔GPU bit-parity ≤ 1e-3 on the golden set (enforced in T06)
- [x] FSV evidence (readback output / screenshot) attached to PH13 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
