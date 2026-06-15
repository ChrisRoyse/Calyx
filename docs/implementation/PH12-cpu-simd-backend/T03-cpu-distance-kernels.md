# PH12 · T03 — CPU distance kernels (cosine / dot / l2)

| Field | Value |
|---|---|
| **Phase** | PH12 — CPU SIMD Backend |
| **Stage** | S2 — Forge Math Runtime |
| **Crate** | `calyx-forge` |
| **Files** | `crates/calyx-forge/src/cpu/distance.rs` (≤500) |
| **Depends on** | T01 (this phase) |
| **Axioms** | A13, A16 |
| **PRD** | `dbprdplans/13 §3`, `dbprdplans/13 §6`, `dbprdplans/23 §3` |

## Goal

Implement vectorized `cosine`, `dot`, and `l2` distance kernels for the
`CpuBackend` using `wide::f32x16` (AVX-512). Kernels are batched over candidate
blocks (query × N candidates in one call). These are the reference implementations
that PH13's CUDA kernels must agree with within ≤ 1e-3 rel on the golden set, and
that TurboQuant (PH14) and Ward (PH37) consume for agreement/guard scoring.

## Build (checklist of concrete, code-level steps)

- [x] `pub fn cosine_batch(query: &[f32], candidates: &[f32], dim: usize, out: &mut [f32]) -> Result<(), ForgeError>`
  — `candidates` is `n_cands × dim` row-major; `out` is `n_cands` cosines;
  computes `dot(q, c_i) / (‖q‖ · ‖c_i‖)` per candidate; fused normalize+dot in
  one pass (no separate normalization allocation); SIMD dot via `f32x16` lanes
- [x] `pub fn dot_batch(query: &[f32], candidates: &[f32], dim: usize, out: &mut [f32]) -> Result<(), ForgeError>`
  — same layout; raw dot products, no normalization
- [x] `pub fn l2_batch(query: &[f32], candidates: &[f32], dim: usize, out: &mut [f32]) -> Result<(), ForgeError>`
  — squared Euclidean distance `‖q − c_i‖²`; SIMD subtraction + multiply-accumulate
- [x] All three: shape guard `query.len() == dim`, `candidates.len() == n_cands * dim`,
  `out.len() == n_cands`; zero-length candidates → empty `out`, no error
- [x] NaN/Inf in query or any candidate row → `ForgeError::NumericalInvariant { op: "cosine_batch" | "dot_batch" | "l2_batch", detail: "input contains non-finite f32" }`
- [x] Zero-norm vector in cosine → `ForgeError::NumericalInvariant { op: "cosine_batch", detail: "zero-norm vector" }` (never silent NaN)
- [x] Fixed SIMD reduction order documented with `// DETERMINISM:` comment (same
  convention as gemm.rs)
- [x] `impl Backend for CpuBackend` delegates `cosine`, `dot`, `l2` to these functions

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: `cosine_batch` with orthogonal vectors (90°) → output = `0.0`; parallel
  vectors → `1.0`; anti-parallel → `-1.0` (exact f32, since inputs are unit vectors)
- [x] unit: `l2_batch` with `q=[0,0]`, `c=[[3,4]]` → `out=[25.0]` (exact integer math)
- [x] proptest: `cosine(a, a) == 1.0` for any random non-zero f32 vector (within 1e-6)
- [x] proptest: `dot_batch` matches scalar reference `Σ q_i·c_i` within 1e-5 for
  random dim-128 vectors (100 candidates)
- [x] edge (≥3): (1) single candidate (`n_cands=1`); (2) `dim=1`; (3) `dim=1536`
  (a real model output dim) — all produce correct output without panic
- [x] fail-closed: query with `f32::INFINITY` → `CALYX_FORGE_NUMERICAL_INVARIANT`;
  zero-norm query in cosine → `CALYX_FORGE_NUMERICAL_INVARIANT`

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `tests/cpu_kernels.rs::cosine_orthogonal_exact` + `cosine_parallel_exact`
  + `l2_pythagorean_exact` on aiwonder
- **Readback:**
  ```
  cargo test -p calyx-forge cpu::distance -- --nocapture 2>&1 | grep -E "PASSED|FAILED|ok"
  ```
- **Prove:** named exact tests PASSED; cosine orthogonal → `0.00000000`, cosine
  parallel → `1.00000000` printed in nocapture; absent: any NaN output or panic

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] CPU↔GPU bit-parity ≤ 1e-3 on the golden set (enforced in T05)
- [x] FSV evidence (readback output / screenshot) attached to the PH12 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
