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

- [ ] `pub struct GemmProblem { pub m: usize, pub k: usize, pub n: usize, pub a_offset: usize, pub b_offset: usize, pub c_offset: usize }`
  — offsets into pre-allocated slab buffers (arena allocator pattern)
- [ ] `pub struct GroupedGemmPlan { problems: Vec<Option<GemmProblem>>, a_slab: CudaSlice<f32>, b_slab: CudaSlice<f32>, c_slab: CudaSlice<f32> }`
  — `Option<GemmProblem>`: `None` = absent slot (skip); `Some` = active lens
- [ ] `pub fn build_grouped_gemm_plan(ctx: &CudaContext, problems: Vec<Option<GemmProblem>>, ...) -> Result<GroupedGemmPlan, ForgeError>`
  — allocate slab buffers; sort `Some` entries by `(k, n)` for cuBLAS perf
  (maintains a mapping back to original slot index for result reconstruction)
- [ ] `pub fn execute_grouped_gemm(ctx: &CudaContext, plan: &GroupedGemmPlan) -> Result<(), ForgeError>`
  — build cuBLAS grouped problem arrays (pointers, dims, alphas, betas);
  call `cublasGemmGroupedBatchedEx` with `CUBLAS_COMPUTE_32F`, alpha=1.0, beta=0.0;
  if `GemmGroupedBatchedEx` unavailable (cuBLAS version check) → fall back to sequential
  `cublasGemmEx` per active problem with a `cargo:warning="GemmGroupedBatchedEx not available; falling back to sequential"`
- [ ] Never write to `c_offset` of a `None` problem — verify with a debug assertion
  that absent slots' output buffers remain at their initial (caller-set) values
- [ ] Expose via `CudaBackend`: `fn grouped_gemm(&self, plan: &mut GroupedGemmPlan) -> Result<(), ForgeError>`

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: grouped GEMM with 1 problem = single GEMM; result matches `gemm_cublas` within 1e-5
- [ ] unit: grouped GEMM with 3 problems of sizes (2×2×2), (4×3×2), (1×5×3) —
  each result matches the individually computed matmul within 1e-4
- [ ] proptest: for N random square problems (N ∈ 1..8, dim ∈ 2..16), grouped GEMM
  result == per-problem loop result within 1e-4 for all elements
- [ ] edge (≥3): (1) all-`None` plan → no kernel launch, no error; (2) one `None`
  in the middle of active problems → output for active problems unchanged;
  (3) N=1 problem
- [ ] fail-closed: mismatched slab buffer size → `ForgeError::ShapeMismatch` at plan build time

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `grouped_gemm_tests::grouped_equals_per_loop` on aiwonder
- **Readback:**
  ```bash
  source $CALYX_HOME/repo/env.sh
  cargo test -p calyx-forge --features cuda grouped_gemm -- --nocapture 2>&1 \
    | grep -E "per_loop|grouped|max_err|PASSED|FAILED"
  ```
- **Prove:** `grouped_equals_per_loop` PASSED printing `max_err=X.XXe-Y` (≤ 1e-4);
  absent-slot test PASSED; absent: any output modification in `None` slot buffers

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] CPU↔GPU bit-parity ≤ 1e-3 on the golden set (grouped GEMM == per-loop GEMM)
- [ ] FSV evidence attached to PH15 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
