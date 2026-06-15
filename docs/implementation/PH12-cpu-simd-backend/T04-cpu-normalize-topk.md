# PH12 · T04 — CPU normalize + topk

| Field | Value |
|---|---|
| **Phase** | PH12 — CPU SIMD Backend |
| **Stage** | S2 — Forge Math Runtime |
| **Crate** | `calyx-forge` |
| **Files** | `crates/calyx-forge/src/cpu/normalize.rs` (≤500), `crates/calyx-forge/src/cpu/topk.rs` (≤500) |
| **Depends on** | T01 (this phase) |
| **Axioms** | A13, A16 |
| **PRD** | `dbprdplans/13 §3`, `dbprdplans/13 §6` |

## Goal

Implement in-place L2 normalization (fused norm-compute + scale, NaN/Inf guarded)
and index-stable min-heap topk for the `CpuBackend`. Both are called on every
ingest path and ANN rerank path; determinism and fail-closed behaviour are
mandatory. `topk` is the CPU reference that PH13's GPU bitonic sort must agree
with on the golden set.

## Build (checklist of concrete, code-level steps)

- [x] `src/cpu/normalize.rs`: `pub fn normalize_f32(vecs: &mut [f32], dim: usize) -> Result<(), ForgeError>`
  — in-place per-row L2 normalize of an `n × dim` matrix stored row-major;
  compute `norm = sqrt(Σ v_i²)` via SIMD `f32x16` reduce; check `norm.is_finite()
  && norm > 0.0` before dividing — zero or non-finite norm → `ForgeError::NumericalInvariant
  { op: "normalize", detail: "zero or non-finite L2 norm at row {r}" }`;
  SIMD scale pass (multiply by `1/norm`)
- [x] Finite-check sweep on input before any computation: first non-finite element
  → `ForgeError::NumericalInvariant { op: "normalize", detail: "input[{i}] is non-finite" }`
- [x] `impl Backend for CpuBackend`: `normalize` delegates to `normalize_f32`
- [x] `src/cpu/topk.rs`: `pub fn topk_f32(scores: &[f32], k: usize) -> Result<Vec<(usize, f32)>, ForgeError>`
  — min-heap of capacity k; iterate all scores once; tie-breaking by **lower
  index wins** (index-stable) so results are fully deterministic; returns a `Vec`
  sorted descending by score
- [x] `k == 0` → return empty vec (not an error); `k >= scores.len()` → return all
  scores sorted; `scores` empty + `k > 0` → return empty vec
- [x] Any `f32::NAN` in `scores` → `ForgeError::NumericalInvariant { op: "topk", detail: "NaN in score at index {i}" }`
- [x] `impl Backend for CpuBackend`: `topk` delegates to `topk_f32`

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: `normalize_f32` on `[[3.0, 4.0]]` → `[[0.6, 0.8]]` (exact: 3-4-5 right triangle)
- [x] unit: `topk_f32([0.1, 0.9, 0.5, 0.9], 2)` → `[(1, 0.9), (3, 0.9)]` (lower index wins on ties)
- [x] proptest: `normalize(v)` → `‖v‖ == 1.0` within 1e-6 for random non-zero vectors
- [x] proptest: topk result is a subset of the input, sorted descending, length = min(k, n)
- [x] edge (≥3): (1) single-element vec topk k=1; (2) all-equal scores (tie-break
  by index produces indices 0,1,…,k-1); (3) `dim=1` normalization
- [x] fail-closed: zero-vector → `CALYX_FORGE_NUMERICAL_INVARIANT`; NaN in scores → `CALYX_FORGE_NUMERICAL_INVARIANT`

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `tests/cpu_kernels.rs::normalize_345_exact` + `topk_tie_break_deterministic` on aiwonder
- **Readback:**
  ```
  cargo test -p calyx-forge cpu::normalize cpu::topk -- --nocapture 2>&1 | grep -E "PASSED|FAILED"
  ```
- **Prove:** `normalize_345_exact` prints `[0.6, 0.8]`; `topk_tie_break_deterministic`
  prints `[(1, 0.9), (3, 0.9)]`; absent: panics, NaN in output

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] CPU↔GPU bit-parity ≤ 1e-3 on the golden set (enforced in T05)
- [x] FSV evidence (readback output / screenshot) attached to the PH12 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
