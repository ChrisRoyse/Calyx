# PH15 · T05 — N-invariance FSV + per-matmul-loop equivalence

| Field | Value |
|---|---|
| **Phase** | PH15 — MXFP4/Microscaling + Grouped GEMM |
| **Stage** | S2 — Forge Math Runtime |
| **Crate** | `calyx-forge` |
| **Files** | `crates/calyx-forge/tests/grouped_gemm_tests.rs` (≤500) |
| **Depends on** | T03, T04 (this phase) |
| **Axioms** | A13, A25 |
| **PRD** | `dbprdplans/23 §3`, `dbprdplans/12_STAGE2_FORGE.md PH15 FSV gate` |

## Goal

Write the FSV tests for PH15: (1) grouped GEMM result equals per-matmul loop
result element-wise within 1e-4; (2) result is **N-invariant** — adding no-op
lens slots (identity projections) does not change the output for other lenses;
(3) MXFP4 GEMM result is within distortion bound on Assay-safe slots; (4)
partial-bundle (ragged) batch produces correct per-constellation results.

## Build (checklist of concrete, code-level steps)

- [x] Test `grouped_equals_per_loop`: N=5 problems of varied shapes; run grouped;
  run per-loop (sequential `gemm_cublas` per problem); `assert_parity(grouped[i], loop[i], "grouped_gemm", 1e-4)` for all i; print max_err
- [x] Test `grouped_gemm_n_invariant`: create a 3-lens panel; compute result; add
  2 more lens slots that are identity projections (A=I, so output = input);
  rerun grouped on 5 lenses; assert that the first 3 lens outputs are identical
  to the 3-lens run (within 1e-5); print `n_invariant_max_delta=X.XXe-Y`
- [x] Test `mxfp4_within_bound`: for Assay-safe slots, run fp4 GEMM vs f32 GEMM;
  assert `max |fp4_result[i] - f32_result[i]| / |f32_result[i]| ≤ 0.05`
  (5% relative bound on quant-safe slots); print `fp4_within_bound=X.XXe-Y`
- [x] Test `partial_bundle_correct`: `RaggedBatch` with 4 constellations × 3 slots,
  where constellation 2 has slot 1 absent; run batch; verify that constellations
  0, 1, 3 and constellation 2's slots 0, 2 match per-loop results within 1e-4
- [x] All tests `#[cfg_attr(not(feature="cuda"), ignore)]`

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] All inputs seeded via `ChaCha8Rng(seed=0xCALYX15)` for full determinism
- [x] `grouped_equals_per_loop`: assert all 5 problems' parity within 1e-4; print per-problem max_err
- [x] `grouped_gemm_n_invariant`: assert `n_invariant_max_delta < 1e-5` (exact invariance
  within floating-point round-off)
- [x] edge (≥3): (1) N=1 (degenerate group); (2) N=32 (large group); (3) mixed fp4 and f32 problems in same batch → fp4 problems within 5%, f32 within 1e-4
- [x] fail-closed: mismatched output buffer for a `None` slot → debug_assert fires
  (see T04); in release build, output is `None` (not zero)

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `grouped_gemm_tests::grouped_equals_per_loop` + `grouped_gemm_n_invariant`
  + `mxfp4_within_bound` + `partial_bundle_correct` on aiwonder RTX 5090
- **Readback:**
  ```bash
  source $CALYX_HOME/repo/env.sh
  cargo test -p calyx-forge --features cuda grouped_gemm_tests -- --nocapture 2>&1 \
    | grep -E "n_invariant|per_loop|fp4_within|partial|PASSED|FAILED" \
    | tee /tmp/ph15_fsv.txt
  cat /tmp/ph15_fsv.txt
  ```
- **Prove:** all 4 named tests PASSED; `n_invariant_max_delta=X.XXe-Y` where Y ≥ 5
  (i.e., delta < 1e-5); `fp4_within_bound` prints ratio ≤ 0.05; `partial_bundle_correct`
  PASSED showing absent slot = None; `/tmp/ph15_fsv.txt` attached to PH15 issue

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] **Grouped GEMM result == per-matmul loop** (T05 `grouped_equals_per_loop` is the proof)
- [x] **N-invariant** (`grouped_gemm_n_invariant` is the proof)
- [x] **FP4 within bound on safe slots** (`mxfp4_within_bound` is the proof)
- [x] **Partial-bundle batch correct** (`partial_bundle_correct` is the proof)
- [x] CPU↔GPU bit-parity ≤ 1e-3 on the golden set
- [x] FSV evidence (`/tmp/ph15_fsv.txt`) attached to PH15 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
