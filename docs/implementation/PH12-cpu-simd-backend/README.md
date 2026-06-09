# PH12 — CPU SIMD Backend

> **DONE — PH12 shipped on aiwonder.**
> The Backend trait, CPU kernels, golden fixtures, and fail-closed guards are in
> `calyx-forge`. Continue Stage 2 at PH13 (`PH13-cuda-sm120-bitparity/`).

**Stage:** S2 — Forge Math Runtime  ·  **Crate:** `calyx-forge`  ·
**PRD roadmap:** P1  ·  **Axioms:** A13, A16

## Objective

Implement the reference and production CPU path for `calyx-forge`: vectorized
`gemm`, `cosine`/`dot`/`l2`, `normalize`, and `topk` kernels using
`wide`/`std::simd` targeting AVX-512 on the aiwonder Ryzen (16c/32t, AVX-512
capable). Define the `Backend` trait so the CUDA path (PH13) and future backends
plug in without changing call sites. Build seeded golden-vector fixtures and
validate against numpy/BLAS reference outputs to establish the numeric ground
truth all later phases must agree with.

## Dependencies

- **Phases:** PH04 (`calyx-core` structs + traits — `Slot`, `Constellation`,
  error catalog, `CALYX_*` codes, `Clock` trait)
- **Provides for:** PH13 (GPU path implements same `Backend` trait + compares
  against golden set produced here), PH14 (TurboQuant consumes `gemm`/`dot`),
  PH15 (MXFP4 grouped GEMM builds on CPU reference), PH16 (autotune microbench
  infrastructure), PH17 (lens runtime calls Forge CPU path on embedded vaults)

## Current state

PH12 is implemented in `crates/calyx-forge`:

- `Backend`, `BackendKind`, `BestConfig`, `DeviceInfo`, and `ForgeError` define
  the Forge backend contract and typed fail-closed error surface.
- `CpuBackend` implements deterministic CPU `gemm`, `cosine`, `dot`, `l2`,
  `normalize`, and `topk`.
- Shared CPU guards reject non-finite inputs and invalid norms with
  `CALYX_FORGE_NUMERICAL_INVARIANT` plus remediation text.
- Seeded numpy/scipy golden fixtures live under
  `crates/calyx-forge/tests/golden/`; `tests/cpu_kernels.rs` validates the
  kernels against those source-of-truth bytes.

The CUDA path is intentionally deferred to PH13.

## Deliverables (file plan, each ≤500 lines)

| File | Responsibility |
|---|---|
| `src/backend.rs` | `Backend` trait: `gemm`, `cosine`, `dot`, `l2`, `normalize`, `topk`, `device_info`; `BackendKind` enum (Cpu, Cuda); `BestConfig` type |
| `src/cpu/mod.rs` | `CpuBackend` impl, dispatch to AVX-512/AVX2 via `wide`/`std::simd` feature gate |
| `src/cpu/gemm.rs` | SIMD-vectorized GEMM (column-major, f32); deterministic reduction order |
| `src/cpu/distance.rs` | `cosine`, `dot`, `l2` — fused normalize+dot; batched over candidate blocks |
| `src/cpu/normalize.rs` | L2 normalize, NaN/Inf guards → `CALYX_FORGE_NUMERICAL_INVARIANT` |
| `src/cpu/topk.rs` | heap-based topk, deterministic on ties (index-stable) |
| `src/error.rs` | `ForgeError` → `CALYX_FORGE_NUMERICAL_INVARIANT`, `CALYX_FORGE_DEVICE_UNAVAILABLE`; maps to `calyx-core` error catalog |
| `tests/golden/` | seeded golden-vector fixtures (f32 `.bin` + `.json` metadata); numpy/BLAS reference outputs |
| `tests/cpu_kernels.rs` | unit + proptest + edge tests against golden set |

## Tasks (atomic — all must pass for the phase to be DONE)

| Card | Title | Depends |
|---|---|---|
| T01 | Backend trait + error types | — |
| T02 | CPU GEMM kernel (AVX-512, deterministic) | T01 |
| T03 | CPU distance kernels (cosine / dot / l2) | T01 |
| T04 | CPU normalize + topk | T01 |
| T05 | Golden-vector fixtures + numpy reference validation | T02, T03, T04 |
| T06 | NaN/Inf guards + fail-closed paths | T01, T02, T03, T04 |

## FSV exit gate (the phase is DONE only when this is byte-proven on aiwonder)

Run on aiwonder (RTX 5090, Ryzen AVX-512):

```
cd $CALYX_HOME/repo
source ~/.cargo/env
cargo test -p calyx-forge -- --nocapture 2>&1 | tee /tmp/ph12_fsv.txt
```

Proof: `tests/cpu_kernels.rs::golden_cosine_matches_numpy` passes — the test
reads the pre-built `.bin` golden file, computes cosine via `CpuBackend`, and
asserts `|computed - reference| ≤ 1e-5` element-wise. The readback command:

```
grep "golden_cosine\|golden_gemm\|golden_topk\|PASSED" /tmp/ph12_fsv.txt
xxd tests/golden/cosine_ref.bin | head -4   # first bytes of reference
```

NaN input test must print `CALYX_FORGE_NUMERICAL_INVARIANT` in the error output
(grep for it in the test output). No panics, no silent zeros.

## Completion evidence

PH12 closed through GitHub issues #71-#76:

| Issue | Commit | FSV root on aiwonder |
|---|---|---|
| #71 Backend trait + error types | `fcb4d4e` | `/home/croyse/calyx/data/fsv-q71-20260607115027` |
| #72 CPU GEMM kernel | `998000d` | `/home/croyse/calyx/data/fsv-q72-20260607115838` |
| #73 CPU distance kernels | `df6ddf9` | `/home/croyse/calyx/data/fsv-q73-20260607120318` |
| #74 CPU normalize + topk | `fb38a00` | `/home/croyse/calyx/data/fsv-q74-20260607120828` |
| #75 Golden fixtures | `eb6e00d`, `545abff` | `/home/croyse/calyx/data/fsv-q75-20260607121521` |
| #76 NaN/Inf guards | `807a2af` | `/home/croyse/calyx/data/fsv-q76-20260607122351` |

The Stage 2 epic (#70) is complete and FSV-signed-off. PH12 evidence is listed
above; the aggregate Stage 2 status is tracked in #23 and
`12_STAGE2_FORGE.md`. Later stages have since consumed Forge successfully; the
current active frontier is tracked in `../03_PHASE_MAP.md`.

## Risks / landmines

- **AVX-512 detection:** `wide` / `std::simd` dispatch requires the binary to be
  built on aiwonder itself (native, `-C target-cpu=native`); cross-built binaries
  silently fall back to scalar. Mitigation: `RUSTFLAGS="-C target-cpu=native"` in
  `env.sh`; a test asserts `is_x86_feature_detected!("avx512f")` at runtime.
- **Deterministic reduction order:** floating-point reductions are order-dependent.
  Fix reduction order at the source level (sequential within a SIMD lane, then
  fixed horizontal add order) and document it; any change is a breaking API change.
- **Golden file format:** use little-endian f32 raw binary + a JSON sidecar for
  shape/seed. Numpy `tofile()` writes the same format. Pin numpy version in the
  generator script to avoid silent precision drift.
- **`wide` crate SIMD width:** AVX-512 is 512-bit (16×f32); `wide::f32x16` is the
  right type. Fallback to `f32x8` (AVX2) must produce the same numeric answer
  (determinism contract).
