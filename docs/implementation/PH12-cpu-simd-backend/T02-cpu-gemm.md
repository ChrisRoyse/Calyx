# PH12 · T02 — CPU GEMM kernel (AVX-512, deterministic)

| Field | Value |
|---|---|
| **Phase** | PH12 — CPU SIMD Backend |
| **Stage** | S2 — Forge Math Runtime |
| **Crate** | `calyx-forge` |
| **Files** | `crates/calyx-forge/src/cpu/gemm.rs` (≤500), `crates/calyx-forge/src/cpu/mod.rs` (≤500) |
| **Depends on** | T01 (this phase) |
| **Axioms** | A13, A16 |
| **PRD** | `dbprdplans/13 §2/§3`, `dbprdplans/23 §3` |

## Goal

Implement a vectorized, deterministic single-precision GEMM (`C = A × B`, column-
major f32) for the `CpuBackend`, targeting AVX-512 on the aiwonder Ryzen via
`wide::f32x16`. Deterministic reduction order is mandatory — any reorder is a
breaking change to the bit-parity contract that PH13 must satisfy. This kernel is
the CPU reference that the CUDA path (PH13) must agree with within ≤ 1e-3 rel.

## Build (checklist of concrete, code-level steps)

- [x] `src/cpu/mod.rs`: declare `pub struct CpuBackend;`; `impl Backend for CpuBackend`
  delegating to module functions; include `#[cfg(target_arch="x86_64")]`
  `is_x86_feature_detected!("avx512f")` assertion in `new()` that logs a warning
  (not an error) if AVX-512 is absent and falls back to `f32x8` (AVX2)
- [x] `src/cpu/gemm.rs`: `pub fn gemm_f32(a: &[f32], b: &[f32], m: usize, k: usize, n: usize, out: &mut [f32]) -> Result<(), ForgeError>`
  — column-major layout (A is m×k, B is k×n, C is m×n); tiled loop with tile
  size `TILE_M=64, TILE_K=64` (constants, not runtime config at this phase)
- [x] Inner micro-kernel uses `wide::f32x16` for the k-reduction; horizontal add
  order is **fixed**: sequential lane-0 through lane-15 then `f32x16::reduce_add()`;
  this exact order is documented in a `// DETERMINISM:` comment beside the reduce
- [x] Shape guards: `a.len() == m*k`, `b.len() == k*n`, `out.len() == m*n` —
  mismatch → `ForgeError::ShapeMismatch`; any NaN/Inf in `a` or `b` detected via
  `f32::is_finite` sweep before compute → `ForgeError::NumericalInvariant`
- [x] `CpuBackend::gemm(...)` delegates to `gemm_f32` and is the `Backend` trait impl

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: 2×2 matmul with known values `A=[[1,2],[3,4]]`, `B=[[5,6],[7,8]]` →
  assert `C == [[19,22],[43,50]]` (exact f32 equality for small integers)
- [x] unit: 1×1 matmul (k=1) and 1×k×1 dot-product shape — degenerate cases
- [x] proptest: `gemm(A, I) == A` (identity matrix right-multiply) for random
  f32 matrices sized 1–64 each dim; tolerance ≤ 1e-5 per element
- [x] proptest: `gemm(A, B)[i,j]` equals the scalar dot product of row i of A
  and column j of B (within 1e-5) for random 4×4 matrices
- [x] edge (≥3): (1) m=0 (empty output, no panic); (2) k=1 (outer product); (3)
  maximum TILE edge (m=64, k=64, n=64) — shape exactly one tile
- [x] fail-closed: input containing `f32::NAN` → `ForgeError::NumericalInvariant`;
  `out.len()` too small → `ForgeError::ShapeMismatch`

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `tests/cpu_kernels.rs::gemm_2x2_exact` + `gemm_identity_proptest` on aiwonder
- **Readback:**
  ```
  cargo test -p calyx-forge cpu::gemm -- --nocapture 2>&1 | grep -E "PASSED|FAILED|ok"
  ```
- **Prove:** both named tests PASSED; the 2×2 exact test shows `C[0]=19.0, C[3]=50.0`
  in nocapture output; AVX-512 detection log line present (or AVX2 fallback note);
  absent: any NaN in output, any panic

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] CPU↔GPU bit-parity ≤ 1e-3 on the golden set (proven in T05; this card
      establishes the CPU reference that makes that possible)
- [x] FSV evidence (readback output / screenshot) attached to the PH12 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
