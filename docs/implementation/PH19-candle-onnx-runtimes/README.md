# PH19 — candle-local + onnx runtimes

**Stage:** S3 — Registry / Lenses  ·  **Crate:** `calyx-registry`  ·
**PRD roadmap:** P2  ·  **Axioms:** A4

## Objective

Run lens neural networks locally — either via `candle` on sm_120 (RTX 5090)
or via ONNX Runtime with the CUDA EP — so that embedded vaults and bespoke
lenses can operate without an HTTP hop to a TEI service. Load weights from
`CALYX_HOME/.hf-cache` (populated by the HF Hub client, token from
`CALYX_HF_TOKEN` env var). All outputs must pass the frozen contract from PH18:
finite, unit-norm where declared, correct dimension.

## Dependencies

- **Phases:** PH18 (frozen contract — all four guards wired), PH13 (CUDA
  sm_120 backend — cudarc/candle device available)
- **Provides for:** PH20 (hot-swap can add candle/ONNX lenses), PH21
  (capability card costs measured against real local runtimes)

## Current state (build off what exists)

`calyx-registry` has PH17+PH18: Registry, five runtime variant types declared,
frozen contract enforced. `runtime/candle.rs` and `runtime/onnx.rs` are empty
stubs (their variant arms exist in `LensRuntime` but the impls are `unimplemented!()`).
Greenfield fill-in.

**aiwonder:** `CALYX_HOME/.hf-cache` stores HF models. `CALYX_HF_TOKEN` in
env. Candle with cudarc targets sm_120. ORT CUDA EP prebuilt for MSVC 14.44
(see memory `synapse-mcp-build-windows.md` for toolchain notes — applicable
to Windows dev; aiwonder is Linux/CUDA).

## Deliverables (file plan, each ≤500 lines)

| File | Responsibility |
|---|---|
| `crates/calyx-registry/src/runtime/candle.rs` | `CandleLocalLens`: weight loading from HF cache, forward pass via candle, L2 normalize |
| `crates/calyx-registry/src/runtime/onnx.rs` | `OnnxLens`: load `.onnx` from HF cache, run ORT CUDA EP, normalize |
| `crates/calyx-registry/src/hf_cache.rs` | HF cache resolver: `CALYX_HOME/.hf-cache/<model-id>/<filename>` path builder |

## Tasks (atomic — all must pass for the phase to be DONE)

| Card | Title | Depends |
|---|---|---|
| T01 | HF cache resolver + weight path builder | — |
| T02 | CandleLocalLens runtime | T01 |
| T03 | OnnxLens runtime (ORT CUDA EP) | T01 |
| T04 | Dim guard + unit-norm for local runtimes | T02, T03 |
| T05 | Integration: candle + ONNX each produce valid vectors on aiwonder | T04 |

## FSV exit gate (the phase is DONE only when this is byte-proven on aiwonder)

1. `CandleLocalLens` loads a small real model from `.hf-cache`; measures a
   text input; returns `SlotVector::Dense` where all values are finite and L2
   norm ≈ 1.0 — print norm to stdout.
2. `OnnxLens` does the same with an `.onnx` file from `.hf-cache`.
3. Declare a lens with `SlotShape::Dense(128)` but runtime returns `Dense(768)`
   → `CALYX_LENS_DIM_MISMATCH` fires — test output shows the code.
4. `.hf-cache` directory existence confirmed:
   `ls $CALYX_HOME/.hf-cache/<model-id>/` shows the weight file.

Readback: `cargo test -p calyx-registry -- --include-ignored --nocapture 2>&1
| grep -E 'norm|MISMATCH|hf.cache'` on aiwonder; output attached to PH19
GitHub issue.

## Risks / landmines

- **sm_120 availability at test time:** candle tests that use the GPU are
  `#[ignore]` by default; run them explicitly on aiwonder with
  `--include-ignored`.
- **ORT CUDA EP version pinning:** ORT binary must match the CUDA 13.2 on
  aiwonder; pin the version in `Cargo.toml` and document it.
- **HF download on first run:** the integration test may trigger an HF
  download if the model is not yet in `.hf-cache`; this is acceptable but
  must not block the build — guard with `#[ignore]` and a clear comment.
- **Weight size:** use the smallest available BERT/GTE variant (≤150 MB) for
  candle integration tests to avoid long build/test times.
