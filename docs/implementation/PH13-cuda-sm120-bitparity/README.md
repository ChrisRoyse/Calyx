# PH13 — CUDA sm_120 Backend + Bit-Parity

**Stage:** S2 — Forge Math Runtime  ·  **Crate:** `calyx-forge`  ·
**PRD roadmap:** P1  ·  **Axioms:** A13, A16

## Objective

Add the GPU compute path to `calyx-forge`: CUDA kernels targeting sm_120
(RTX 5090, Blackwell GB202, CUDA 13.3) implementing the same `Backend` trait
defined in PH12. Deliver cudarc/cuBLAS-backed GEMM, fused cosine/topk kernels via
`cudarc`, PTX+cubin for sm_120 with a JIT fallback, and a determinism mode that
pins reductions for FSV replay. The phase's defining gate is **CPU↔GPU ≤ 1e-3
rel** on the PH12 golden set and matmul within **10% of cuBLAS** throughput on
sm_120. `CALYX_FORGE_DEVICE_UNAVAILABLE` must be returned on any CUDA init
failure in server mode — no silent CPU fallback.

## Dependencies

- **Phases:** PH12 (CPU Backend, `Backend` trait, golden-vector fixtures, error
  types — must be DONE)
- **Provides for:** PH14 (TurboQuant rotate/QJL kernels run on GPU path), PH15
  (MXFP4 GEMM and grouped GEMM extend the CUDA backend), PH16 (autotune
  microbench hits GPU path), PH17 (lens runtime on embedded vaults may use GPU),
  PH37 (Ward Gτ cosine gate uses GPU cosine kernel), PH57 (VRAM budgeter wraps
  the CUDA backend)

## Current state (build off what exists)

`calyx-forge` has the `Backend` trait, `CpuBackend`, error types, golden
fixtures, CUDA context/device handling, cudarc/cuBLAS GEMM, fused distance/topk
paths, and a CPU↔GPU parity suite. Build and FSV run natively on aiwonder with
`source $CALYX_HOME/repo/env.sh`; no cross-build. sm_120 requires CUDA 13.3
(`nvcc -arch=sm_120`); older CUDA versions produce `unsupported GPU architecture`
errors — pin explicitly.

## Deliverables (file plan, each ≤500 lines)

| File | Responsibility |
|---|---|
| `src/cuda/mod.rs` | `CudaBackend` struct; `impl Backend for CudaBackend`; init + `CALYX_FORGE_DEVICE_UNAVAILABLE` on fail |
| `src/cuda/context.rs` | CUDA context init via `cudarc`; device query; determinism mode flag |
| `src/cuda/gemm.rs` | cudarc/cuBLAS GEMM wrapper (f32, bf16); sm_120 handle init; 10%-of-cuBLAS perf gate |
| `src/cuda/distance.rs` | Fused cosine/dot/l2 kernels; batched over candidate blocks; `cudarc` dispatch |
| `src/cuda/topk.rs` | GPU bitonic sort topk; deterministic (fixed sort direction per tie) |
| `src/cuda/kernels/` | `.ptx` / `.cu` source for sm_120 fused distance + topk; compiled at build time |
| `build.rs` | `nvcc -arch=sm_120` compilation of `.cu` kernels; embed PTX bytes; JIT fallback |
| `tests/cuda_parity.rs` | CPU↔GPU parity tests against PH12 golden fixtures; ≤1e-3 rel gate |

## Tasks (atomic — all must pass for the phase to be DONE)

| Card | Title | Depends |
|---|---|---|
| T01 | CUDA context init + device query + DEVICE_UNAVAILABLE | — |
| T02 | build.rs: nvcc sm_120 compilation + PTX embed | T01 |
| T03 | cudarc/cuBLAS GEMM wrapper + 10%-of-cuBLAS perf gate | T02 |
| T04 | Fused GPU distance kernels (cosine / dot / l2) | T02 |
| T05 | GPU bitonic topk | T02 |
| T06 | CPU↔GPU bit-parity suite against golden set | T03, T04, T05 |

## FSV exit gate (the phase is DONE only when this is byte-proven on aiwonder)

Run on aiwonder (RTX 5090, sm_120, CUDA 13.3):

```bash
source $CALYX_HOME/repo/env.sh
cargo test -p calyx-forge --features cuda cuda -- --nocapture 2>&1 | tee /tmp/ph13_fsv.txt

# Parity check:
grep "parity\|rel_err\|PASSED\|FAILED" /tmp/ph13_fsv.txt

# Timing check (matmul 10% gate):
grep "cuBLAS\|forge_gemm\|ratio" /tmp/ph13_fsv.txt
```

Proof: `cuda_parity::golden_cosine_parity` and `cuda_parity::golden_gemm_parity`
PASSED with max rel error ≤ 1e-3 printed; `cuda_gemm::perf_vs_cublas` prints
`ratio = X.XX` where X.XX ≥ 0.90 (Forge throughput is at least 90% of cuBLAS).
`CALYX_FORGE_DEVICE_UNAVAILABLE`
appears in the init-fail path test output.

## Risks / landmines

- **sm_120 / CUDA 13.3 only on aiwonder:** never run GPU tests on Windows dev
  machine (no GPU); all GPU-touching code is feature-gated `#[cfg(feature="cuda")]`
  so `cargo check` passes everywhere; tests are `#[cfg_attr(not(feature="cuda"), ignore)]`.
- **cudarc API surface:** cudarc 0.12.x broke several APIs; pin the exact version in
  `Cargo.toml` (`cudarc = { version = "=0.12.x", features = ["cuda-12050"] }` — pick
  the most recent that compiles against CUDA 13.3 headers).
- **JIT fallback PTX:** embed PTX bytes (not cubin) so the driver JITs to the running
  SM; ship cubin as the fast path. On driver upgrade, the JIT path keeps working.
- **Determinism mode:** GPU reductions (e.g. warp shuffles) are non-deterministic
  without explicit control. Determinism mode uses sequential reduction within a block
  (`--use_fast_math` disabled); document this with a `// DETERMINISM:` comment.
- **VRAM pressure:** aiwonder has resident TEI containers. Initialize CUDA with a
  soft VRAM cap; if `cuMemGetInfo` shows < 4 GB free → log a warning and return
  `CALYX_FORGE_DEVICE_UNAVAILABLE` rather than OOMing the TEI containers.
