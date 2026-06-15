# PH13 · T06 — CPU↔GPU bit-parity suite against golden set

| Field | Value |
|---|---|
| **Phase** | PH13 — CUDA sm_120 Backend + Bit-Parity |
| **Stage** | S2 — Forge Math Runtime |
| **Crate** | `calyx-forge` |
| **Files** | `crates/calyx-forge/tests/cuda_parity.rs` (≤500) |
| **Depends on** | T03, T04, T05 (this phase) · PH12 T05 (golden fixtures) |
| **Axioms** | A13 |
| **PRD** | `dbprdplans/13 §2/§6`, `dbprdplans/19 §4` |

## Goal

Write the definitive CPU↔GPU parity test suite that reads the PH12 golden
fixtures and asserts: (1) `CudaBackend` outputs agree with `CpuBackend` outputs
within **≤ 1e-3 rel** for all of `gemm`, `cosine`, `dot`, `l2`, `topk`; and (2)
matmul throughput is within **10% of cuBLAS** baseline on sm_120. This is the
FSV gate for PH13 — no other evidence suffices.

## Build (checklist of concrete, code-level steps)

Post-sweep #307 clarification: parity uses `<= 1e-3` relative error as the
primary gate and a named `<= 1e-6` absolute floor for near-zero cancellation
cells. The FSV readback must persist both worst relative and worst absolute
locations so this is visible, not a hidden tolerance change.

- [x] `tests/cuda_parity.rs`: import `calyx_forge::{CpuBackend, CudaBackend, Backend}`;
  load golden fixtures via `load_golden_f32` (same helper as PH12 T05)
- [x] `fn max_rel_err(a: &[f32], b: &[f32]) -> f32` — element-wise
  `|a_i - b_i| / (|b_i| + 1e-8)`; returns max across all elements
- [x] `fn assert_parity(cpu: &[f32], gpu: &[f32], op: &str, tol: f32)` — if
  `max_rel_err > tol` → panic with message:
  `"PARITY FAIL op={op} max_rel_err={err:.2e} > tol={tol:.2e} at index {worst_idx} cpu={cpu_val} gpu={gpu_val}"`
- [x] Test `golden_gemm_parity`: CPU gemm on golden A/B → `cpu_C`; GPU gemm on
  same → `gpu_C`; `assert_parity(cpu_C, gpu_C, "gemm", 1e-3)`
- [x] Test `golden_cosine_parity`: CPU cosine_batch → `cpu_cos`; GPU → `gpu_cos`;
  `assert_parity(cpu_cos, gpu_cos, "cosine", 1e-3)`
- [x] Test `golden_dot_parity`: same for dot
- [x] Test `golden_l2_parity`: same for l2
- [x] Test `golden_topk_parity`: CPU topk indices == GPU topk indices (exact int match
  — any index mismatch at same rank → FAIL with both index lists printed)
- [x] Test `perf_vs_cublas`: `bench_gemm_cublas(512,512,512)` vs `bench_gemm_reference_cublas(512,512,512)`;
  ratio ≥ 0.90 or FAIL with `"forge_ratio={ratio:.3} < 0.90 (10% cuBLAS gate) on sm_120"`
- [x] All tests `#[cfg_attr(not(feature="cuda"), ignore)]` so they are skipped on non-CUDA builds

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: `max_rel_err(&[1.0, 2.0], &[1.0, 2.0])` == 0.0 (identical)
- [x] unit: `max_rel_err(&[1.0], &[1.001])` ≈ 0.001 (within 1e-6 of expected)
- [x] `assert_parity` with a pair that differs by 2e-3 and tol=1e-3 → panics with
  `"PARITY FAIL"` in the message
- [x] proptest: `max_rel_err(x, x)` == 0.0 for all finite non-zero x
- [x] edge (≥3): (1) parity on 1-element arrays; (2) parity where one element is
  near-zero (denominator clamp to 1e-8); (3) topk parity with tied scores
- [x] fail-closed: `assert_parity` with large error → panic (not just a log) so
  the test harness marks it FAILED

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `tests/cuda_parity.rs` full suite on aiwonder RTX 5090
- **Readback:**
  ```bash
  source $CALYX_HOME/repo/env.sh
  cargo test -p calyx-forge --features cuda -- --nocapture 2>&1 \
    | grep -E "parity|PASSED|FAILED|ratio|rel_err" \
    | tee /tmp/ph13_parity_fsv.txt
  cat /tmp/ph13_parity_fsv.txt
  ```
- **Prove:** every `golden_*_parity` test PASSED; `perf_vs_cublas` PASSED with
  `forge_ratio >= 0.90` printed; absent: any `PARITY FAIL` or `forge_ratio < 0.90`
  line; the file `/tmp/ph13_parity_fsv.txt` is attached to the PH13 GitHub issue

## Done when

#307 adds one required readback artifact here: `cuda-gemm-parity.json` records
worst relative and worst absolute GEMM deltas plus the pass reason (`relative`
or `absolute_near_zero`).

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] **CPU↔GPU bit-parity ≤ 1e-3 rel on the golden set** — this card is the proof
- [x] **matmul within 10% of cuBLAS on sm_120** — `perf_vs_cublas` is the proof
- [x] FSV evidence (`/tmp/ph13_parity_fsv.txt` content / screenshot) attached to PH13 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
