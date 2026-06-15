# PH15 · T03 — Grouped GEMM wrapper (variable-shape problem list)

| Field | Value |
|---|---|
| **Phase** | PH15 — MXFP4/Microscaling + Grouped GEMM |
| **Stage** | S2 — Forge Math Runtime |
| **Crate** | `calyx-forge` |
| **Files** | `crates/calyx-forge/src/cuda/grouped_gemm.rs` (≤500) |
| **Depends on** | PH13 T03 (cuBLASLt GEMM, CudaContext) |
| **Axioms** | A13, A25 |
| **PRD** | `dbprdplans/23 §3`, `dbprdplans/13 §3` |

## Goal

Implement the grouped GEMM wrapper that executes N differently-sized matmuls in
**one kernel launch** via cuBLAS `GemmGroupedBatchedEx` (cuBLAS 12.5+) or CUTLASS
grouped GEMM. The problem list is variable-shape: each entry is `(m_i, k_i, n_i,
ptr_A_i, ptr_B_i, ptr_C_i)`. This makes the whole-panel lens projection one
optimized dispatch regardless of N — cost scales with total work, not launch
overhead × N (`23 §3`).

## Build (checklist of concrete, code-level steps)

- [x] `pub struct GemmProblem { pub m: usize, pub k: usize, pub n: usize, pub a_offset: usize, pub b_offset: usize, pub c_offset: usize }`
  — offsets into pre-allocated slab buffers (arena allocator pattern)
- [x] `pub struct GroupedGemmPlan { problems: Vec<Option<GemmProblem>>, execution_mode: GroupedGemmExecutionMode, a_slab: CudaSlice<f32>, b_slab: CudaSlice<f32>, c_slab: CudaSlice<f32> }`
  — `Option<GemmProblem>`: `None` = absent slot (skip); `Some` = active lens.
  `execution_mode` is the readback-visible verdict: `not_run`,
  `no_active_problems`, `grouped_batched`, or `sequential_fallback`.
- [x] `pub fn build_grouped_gemm_plan(ctx: &CudaContext, problems: Vec<Option<GemmProblem>>, ...) -> Result<GroupedGemmPlan, ForgeError>`
  — allocate slab buffers; sort `Some` entries by `(k, n)` for cuBLAS perf
  (maintains a mapping back to original slot index for result reconstruction)
- [x] `pub fn execute_grouped_gemm(ctx: &CudaContext, plan: &mut GroupedGemmPlan) -> Result<(), ForgeError>`
  — build cuBLAS grouped problem arrays (pointers, dims, alphas, betas);
  call `cublasGemmGroupedBatchedEx` with `CUBLAS_COMPUTE_32F`, alpha=1.0, beta=0.0;
  if `GemmGroupedBatchedEx` is unsupported, ordinary execution falls back to
  sequential cuBLAS and records `execution_mode = sequential_fallback`.
- [x] `pub fn execute_grouped_gemm_strict(...)`: fails closed with
  `CALYX_FORGE_NUMERICAL_INVARIANT` instead of accepting fallback when one
  grouped launch is required.
- [x] Never write to `c_offset` of a `None` problem — verify with a debug assertion
  that absent slots' output buffers remain at their initial (caller-set) values
- [x] Expose via `CudaBackend`: `grouped_gemm` and `grouped_gemm_strict`

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: grouped GEMM with 1 problem = single GEMM; result matches `gemm_cublas` within 1e-5 and strict mode records `grouped_batched`
- [x] unit: grouped GEMM with 3 problems of sizes (2×2×2), (4×3×2), (1×5×3) —
  each result matches the individually computed matmul within 1e-4
- [x] proptest: for N random square problems (N ∈ 1..8, dim ∈ 2..16), grouped GEMM
  result == per-problem loop result within 1e-4 for all elements
- [x] edge (≥3): (1) all-`None` plan → no kernel launch, no error; (2) one `None`
  in the middle of active problems → output for active problems unchanged;
  (3) N=1 problem; #316 FSV also reads `no_active_problems`
- [x] fail-closed: mismatched slab buffer size → `ForgeError::ShapeMismatch` at plan build time

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `grouped_gemm_tests::grouped_equals_per_loop` and
  `ph15_grouped_gemm_execution_mode_aiwonder_fsv` on aiwonder
- **Readback:**
  ```bash
  source $CALYX_HOME/repo/env.sh
  cargo test -p calyx-forge --features cuda grouped_gemm -- --nocapture 2>&1 \
    | grep -E "per_loop|grouped|max_err|PASSED|FAILED"
  ```
- **Prove:** `grouped_equals_per_loop` PASSED printing `max_err=X.XXe-Y` (≤ 1e-4);
  absent-slot test PASSED; absent: any output modification in `None` slot buffers;
  #316 readback prints `allowed_mode`, `strict.mode` or strict error, and
  `empty_mode`.

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] CPU↔GPU bit-parity ≤ 1e-3 on the golden set (grouped GEMM == per-loop GEMM)
- [x] FSV evidence attached to #316 with readback root
  `/home/croyse/calyx/data/fsv-issue316-grouped-gemm-mode-20260608`
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
