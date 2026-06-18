# PH15 — MXFP4/Microscaling + Grouped GEMM

**Stage:** S2 — Forge Math Runtime  ·  **Crate:** `calyx-forge`  ·
**PRD roadmap:** P4b  ·  **Axioms:** A25, A13

## Objective

Implement Blackwell block-scaled compute (MXFP4/NVFP4 with **32-element blocks,
E8M0 power-of-2 scales, fp32 accumulate**; MXFP8 fallback) and **grouped GEMM**
so an N-lens panel projects and scores in one kernel launch regardless of N.
Grouped GEMM uses the in-tree Forge CUDA grouped/ragged surfaces; the current
MXFP4 path uses embedded CUDA fp32 accumulation over MXFP4 blocks. CUTLASS or
native tensor-core MXFP4 promotion is future optimization work, not a current
PH15 claim.
**Absent slots are skipped, never zero-filled** — a mixed-completeness batch
produces the correct per-constellation result. The result of grouped GEMM must be
**invariant to N** (adding a no-op lens does not change the result for the other
lens projections). #316 makes the launch mode part of the source-of-truth
readback: ordinary execution records `grouped_batched`, `sequential_fallback`,
or `no_active_problems`, and strict execution fails loud if true grouped cuBLAS
launch is unavailable.

## Dependencies

- **Phases:** PH14 (TurboQuant must be DONE — MXFP4 per-block scales pair with
  TurboQuant's per-coord scales; the quant safety check follows `23 §4.4`)
- **Provides for:** PH16 (autotune cache operates over grouped GEMM + MXFP4
  shapes), PH17 (lens projection uses grouped GEMM), PH23 (HNSW distance on
  MXFP4-quantized slots), PH37 (Ward Gτ on MXFP4 blocks)

## Current state (build off what exists)

`calyx-forge` has CPU SIMD (PH12), CUDA backend (PH13), TurboQuant (PH14), and
the PH15 MXFP4/MXFP8 codec plus grouped/ragged GEMM surfaces in-tree. Build and
FSV run natively on aiwonder; sm_120 is required for the current MXFP4 CUDA path,
with fallback/edge handling covered by PH15 tests. Native tensor-core/CUTLASS
MXFP4 promotion remains a future optimization. #316 records grouped GEMM launch
mode under `/home/croyse/calyx/data/fsv-issue316-grouped-gemm-mode-20260608`.

## Deliverables (file plan, each ≤500 lines)

| File | Responsibility |
|---|---|
| `src/cuda/mxfp4.rs` | MXFP4/NVFP4 block quantization (32-elt blocks, E8M0 scales), encode/decode, fp32 accumulate |
| `src/cuda/grouped_gemm.rs` | Grouped GEMM wrapper and variable-shape problem list; absent-slot skip |
| `src/quant/mxfp4_codec.rs` | `MxFp4Codec` implementing `Quantizer` trait; Assay-safety gate |
| `tests/mxfp4_tests.rs` | MXFP4 encode/decode + fp32-accumulate parity tests |
| `tests/grouped_gemm_tests.rs` | Grouped GEMM N-invariance + per-matmul-loop equivalence tests |

## Tasks (atomic — all must pass for the phase to be DONE)

| Card | Title | Depends |
|---|---|---|
| T01 | MXFP4 block quantization (32-elt blocks, E8M0 scales) | PH13 T01 |
| T02 | `MxFp4Codec` + fp32-accumulate GEMM path | T01 |
| T03 | Grouped GEMM wrapper (variable-shape problem list) | PH13 T03 |
| T04 | Absent-slot skip + ragged-bundle correctness | T03 |
| T05 | N-invariance FSV + per-matmul-loop equivalence | T03, T04 |

## FSV exit gate (the phase is DONE only when this is byte-proven on aiwonder)

Run on aiwonder (RTX 5090, sm_120, CUDA 13.3):

```bash
source $CALYX_HOME/repo/env.sh
cargo test -p calyx-forge --features cuda grouped_gemm mxfp4 -- --nocapture 2>&1 \
  | tee /tmp/ph15_fsv.txt

grep -E "n_invariant|per_loop_equiv|fp4_within_bound|PASSED|FAILED" /tmp/ph15_fsv.txt
```

Proof: `grouped_gemm_n_invariant` PASSED (result with N=3 lenses == result with N=5
lenses including 2 no-ops); `grouped_gemm_equals_per_loop` PASSED (grouped result
matches per-matmul loop result element-wise within 1e-4); `mxfp4_within_bound` PASSED
(cosine error ≤ ε at fp4 precision for Assay-safe slots); absent-slot test PASSED
(partial bundle → correct per-constellation result, no zero-fill).

#316 readback prints the execution mode and proves strict grouped mode fails
closed rather than silently accepting a sequential fallback.

## Risks / landmines

- **MXFP4 Blackwell-only:** `sm_120` only. The MXFP4 path must be gated on a
  runtime sm version check (`>= 12.0`); if sm < 12.0 → fall back to bf16 with a warning,
  never silently produce wrong results.
- **`GemmGroupedBatchedEx` availability:** requires cuBLAS 12.5+, which ships with
  CUDA 13.x. Confirm with `cublasGetVersion()` at init; if not available → fall back
  to CUTLASS grouped or a host-side loop with a `cargo:warning`.
  #316 supersedes warning-only behavior: ordinary execution exposes
  `execution_mode = sequential_fallback`; strict execution fails loud when true
  grouped launch is required.
- **Ragged problem list:** each (microbatch × slot) entry has its own `m, k, n`;
  the problem list must be sorted by `(k, n)` for cuBLAS perf (documented in cuBLAS
  grouped GEMM guide); unsorted → no wrong answer but slower.
- **Absent slots:** represent missing slots as a `None` entry in the problem list.
  The wrapper must iterate, skip `None`, and never write to the corresponding output
  buffer slot — not zero-fill. This is a correctness invariant, not a performance choice.
- **E8M0 scale format:** E8M0 scales are 8-bit unsigned with exponent-only (no mantissa,
  implicit 1.0 mantissa, power-of-2); each scale covers a 32-element block. Misimplementing
  this (e.g., treating as int8) silently produces wrong scales.
