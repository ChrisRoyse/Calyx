# PH16 · T02 — `microbench` harness (wall-clock, GFLOP/s)

| Field | Value |
|---|---|
| **Phase** | PH16 — Autotune Config Cache |
| **Stage** | S2 — Forge Math Runtime |
| **Crate** | `calyx-forge` |
| **Files** | `crates/calyx-forge/src/autotune/microbench.rs` (≤500) |
| **Depends on** | PH13 T03 (bench_gemm_cublas), PH12 T01 (Backend) |
| **Axioms** | A14, A13 |
| **PRD** | `dbprdplans/12 §4`, `dbprdplans/13 §7` |

## Goal

Implement `microbench(op, config, shape, ctx)` → `BenchResult` that runs the
targeted Forge operation with a specific `BestConfig` on real shapes on aiwonder's
GPU, returns wall-clock elapsed + GFLOP/s, and is the measurement oracle the
explorer (T03) uses to pick winners. Results must be stable (< 10% CV over 5
runs) on aiwonder to be promotable.

## Build (checklist of concrete, code-level steps)

- [x] `pub struct BenchResult { pub gflops: f64, pub elapsed_ms: f64, pub cv_pct: f64 }`
  — `cv_pct` is the coefficient of variation across `iters` runs: `std_dev / mean * 100`
- [x] `pub fn microbench(op: &str, config: &BestConfig, shape: &[usize], ctx: Option<&CudaContext>, iters: u32) -> Result<BenchResult, ForgeError>`
  — dispatch on `op`:
  `"gemm"` → run `CudaBackend.gemm` (or `CpuBackend.gemm`) `iters` times on random f32 data of the given shape; record elapsed via `std::time::Instant` + GPU sync after last call
  `"cosine"` → `cosine_batch` iters times
  `"grouped_gemm"` → `execute_grouped_gemm` iters times
  `"turboquant_encode"` → `TurboQuantCodec.encode` iters times
  unknown op → `ForgeError::Unimplemented { op: op.to_string() }`
- [x] Warm-up: run op once before timing to avoid JIT/paging on first call; warm-up
  is not counted in elapsed
- [x] `cv_pct` computed across `iters` individual timings; if `cv_pct > 20.0` → include
  a `cargo:warning="microbench CV {cv_pct:.1}% > 20% for op={op}; result may be noisy"`
  but still return the result (the explorer decides whether to trust it)
- [x] For GPU ops, call `ctx.inner.synchronize()` before stopping the timer (not just
  after the last call — GPU work may be queued)

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit `#[cfg(feature="cuda")]`: `microbench("gemm", &default_config, &[512,512,512], Some(&ctx), 5)`
  → `gflops > 0.0` and `elapsed_ms > 0.0` and `elapsed_ms < 10000.0` (sanity range)
- [x] unit: `microbench("cosine", &default_config, &[1000, 128], Some(&ctx), 5)` → `BenchResult` with positive values
- [x] unit: `microbench("unknown_op", ...)` → `ForgeError::Unimplemented`
- [x] proptest: running `microbench` twice on the same op/shape → both results positive
  and within 2× of each other (hardware stability bound)
- [x] edge (≥3): (1) `iters=1` (single run, CV=0); (2) CPU path (`ctx=None`, op="gemm")
  works without CUDA; (3) shape `[1,1,1]` (trivial matmul)
- [x] fail-closed: unknown op → `CALYX_FORGE_UNIMPLEMENTED` (via `ForgeError::Unimplemented` Display)

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `autotune_tests::microbench_gemm_returns_positive_gflops` on aiwonder
- **Readback:**
  ```bash
  source $CALYX_HOME/repo/env.sh
  cargo test -p calyx-forge --features cuda autotune::microbench -- --nocapture 2>&1 \
    | grep -E "gflops|elapsed_ms|cv_pct|PASSED|FAILED"
  ```
- **Prove:** test PASSED; output shows `gflops=XXX.X` (positive, reasonable for RTX 5090);
  `cv_pct=X.X` (< 20%); absent: `gflops=0`, panic, or `cv_pct > 20%` warning in normal run

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] CPU↔GPU bit-parity ≤ 1e-3 on the golden set (microbench doesn't check parity
      but the op it measures was validated in PH12–PH15)
- [x] FSV evidence (gflops + cv_pct output) attached to PH16 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
