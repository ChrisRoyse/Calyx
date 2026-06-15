# PH13 · T03 — cuBLASLt GEMM wrapper + 10%-of-cuBLAS perf gate

| Field | Value |
|---|---|
| **Phase** | PH13 — CUDA sm_120 Backend + Bit-Parity |
| **Stage** | S2 — Forge Math Runtime |
| **Crate** | `calyx-forge` |
| **Files** | `crates/calyx-forge/src/cuda/gemm.rs` (≤500) |
| **Depends on** | T01, T02 (this phase) |
| **Axioms** | A13 |
| **PRD** | `dbprdplans/13 §2/§4`, `dbprdplans/23 §3` |

## Goal

Wrap cuBLASLt SGEMM (f32) for the `CudaBackend`, producing results within 10%
of raw cuBLAS throughput on sm_120 for representative matmul shapes (512×512×512,
1024×512×768). This is the big-matmul path used by grouped GEMM (PH15) and
lens projection. Numeric output must agree with the CPU GEMM (PH12 T02) within
≤ 1e-3 rel on the golden set — proven in T06.

## Build (checklist of concrete, code-level steps)

- [x] `src/cuda/gemm.rs`: `pub fn gemm_cublas(ctx: &CudaContext, a: &CudaSlice<f32>, b: &CudaSlice<f32>, m: usize, k: usize, n: usize, out: &mut CudaSlice<f32>) -> Result<(), ForgeError>`
  — uses `cudarc::cublas::CublasHandle`; call `cublasSgemm_v2` with column-major
  layout (`CUBLAS_OP_N`, `CUBLAS_OP_N`), alpha=1.0, beta=0.0
- [x] Handle allocation: `out` is pre-allocated by caller; no `cudaMalloc` inside
  the function (arena allocator contract from `13 §5`)
- [x] On `cublasSgemm` error → `ForgeError::NumericalInvariant { op: "gemm_cublas", detail: "<cublasStatus_t name>", remediation: "..." }`
- [x] `impl Backend for CudaBackend`: `gemm()` copies f32 slices to device via
  `ctx.inner.htod_sync_copy`, calls `gemm_cublas`, copies result back via
  `dtoh_sync_copy`; copy overhead is acceptable for the FSV perf test
- [x] `pub fn bench_gemm_cublas(ctx: &CudaContext, m: usize, k: usize, n: usize, iters: u32) -> f64`
  — returns GFLOP/s: `2.0 * m * k * n * iters / elapsed_ns`; uses `std::time::Instant`
  for wall-clock timing on host (sync after last cuBLAS call)
- [x] `pub fn bench_gemm_reference_cublas(ctx: &CudaContext, m: usize, k: usize, n: usize, iters: u32) -> f64`
  — same timing loop but calls raw `cublasSgemm_v2` directly (no Forge wrapper overhead)
  to establish the cuBLAS baseline

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit `#[cfg(feature="cuda")]`: GEMM identity `A × I = A` for A=4×4 random
  f32; result on GPU matches CPU within 1e-4 element-wise
- [x] unit: `bench_gemm_cublas` for 512×512×512, 10 iters → returns a positive
  finite GFLOP/s value (smoke test; no assertion on speed)
- [x] `perf_vs_cublas` test: `forge_gflops / cublas_gflops >= 0.90` for m=k=n=512;
  if < 0.90 → test FAILS with message `"Forge GEMM ratio={ratio:.3} < 0.90 on sm_120"`
- [x] proptest: random f32 matrices A (m×k) and B (k×n), m,k,n ∈ {1..32} →
  GPU result within 1e-3 rel of CPU result from PH12 `CpuBackend.gemm()`
- [x] edge (≥3): (1) m=1 (row vector × matrix); (2) n=1 (matrix × column vector);
  (3) k=1 (outer product)
- [x] fail-closed: GPU OOM simulated by requesting a 32 GB allocation → `ForgeError::DeviceUnavailable`

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `tests/cuda_parity.rs::perf_vs_cublas` + `gemm_identity_gpu` on aiwonder
- **Readback:**
  ```bash
  source $CALYX_HOME/repo/env.sh
  cargo test -p calyx-forge --features cuda cuda::gemm perf_vs_cublas -- --nocapture 2>&1 \
    | grep -E "ratio|gflops|PASSED|FAILED"
  ```
- **Prove:** `perf_vs_cublas` PASSED; output line contains `ratio=X.XX` where
  X.XX ≥ 0.90; `gemm_identity_gpu` PASSED; absent: any ratio < 0.90 or OOM panic

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] CPU↔GPU bit-parity ≤ 1e-3 on the golden set (enforced in T06)
- [x] FSV evidence (perf ratio + test log / screenshot) attached to PH13 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
