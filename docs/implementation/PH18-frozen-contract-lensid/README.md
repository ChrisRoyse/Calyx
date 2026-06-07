# PH18 — Frozen contract + content-addressed LensId

**Stage:** S3 — Registry / Lenses  ·  **Crate:** `calyx-registry`  ·
**PRD roadmap:** P2  ·  **Axioms:** A4, A16

## Objective

Enforce the frozen instrument contract (`05 §4`) at every registration and
every `measure` call: weights hash must match, output dim/dtype must equal the
declared `SlotShape`, output must be finite and (if declared) unit-norm, and
the lens must not change between two measurements of the same input.
Content-address every lens as `LensId = blake3(name ‖ weights_sha256 ‖
corpus_hash ‖ output_shape)` so identical lenses registered in two separate
vaults always receive the same `LensId`.

## Dependencies

- **Phases:** PH17 (Registry + runtimes exist; determinism probe stub exists)
- **Provides for:** PH19 (candle/ONNX runtimes must pass the same frozen
  contract), PH20 (hot-swap uses `LensId` for dedup on re-register)

## Current state (build off what exists)

`calyx-registry` has T01–T06 from PH17: `Registry`, `LensSpec`, all five
`LensRuntime` variants declared, `AlgorithmicLens`, `TeiHttpLens`, error
codes, and the determinism probe stub. Greenfield for `frozen.rs`.

**aiwonder runtime endpoints:** `:8088` general GTE 768-d, `:8089` reranker,
`:8090` legal. `CALYX_HOME/.hf-cache`, `CALYX_HF_TOKEN` from env.

## Deliverables (file plan, each ≤500 lines)

| File | Responsibility |
|---|---|
| `crates/calyx-registry/src/frozen.rs` | `check_frozen`, `weights_sha256_verify`, `finite_norm_check`, `determinism_probe` (full impl) |
| `crates/calyx-registry/src/lens_id.rs` | `LensId` content-addressing: `blake3(name‖weights_sha256‖corpus_hash‖output_shape)` |
| `crates/calyx-registry/src/lib.rs` | updated `register` to call frozen contract checks |

## Tasks (atomic — all must pass for the phase to be DONE)

| Card | Title | Depends |
|---|---|---|
| T01 | LensId content-addressing (blake3) | — |
| T02 | weights_sha256 frozen-violation guard | T01 |
| T03 | Dim/dtype mismatch guard | T01 |
| T04 | Finite + unit-norm numerical invariant guard | T01 |
| T05 | Full frozen contract enforcement at register + measure | T02, T03, T04 |
| T06 | Cross-vault LensId stability test | T01 |

## FSV exit gate (the phase is DONE only when this is byte-proven on aiwonder)

1. Register a `TeiHttpLens`; swap its `weights_sha256` to a wrong value;
   attempt measure → `CALYX_LENS_FROZEN_VIOLATION` returned, no vector produced.
2. Register a lens declaring `SlotShape::Dense(128)`; runtime returns
   `Dense(768)` → `CALYX_LENS_DIM_MISMATCH`.
3. Register the same `LensSpec` in two `Registry` instances (simulating two
   vaults); `LensId` bytes are identical in both — read with
   `println!("{:x}", lens_id)` and confirm equality.

Readback: `cargo test -p calyx-registry frozen -- --include-ignored --nocapture`
on aiwonder; test output lines showing each CALYX_* code attached to PH18
GitHub issue.

## Risks / landmines

- **blake3 input order must be canonical:** pin the concatenation order as
  `name bytes ‖ weights_sha256 ‖ corpus_hash ‖ output_shape serde-json bytes`;
  document it in code; any reordering silently breaks cross-vault stability.
- **f32 unit-norm tolerance:** TEI returns vectors that may be slightly off
  unit-norm (1.0 ± 1e-5); set tolerance at `1e-4` to avoid spurious
  `CALYX_LENS_NUMERICAL_INVARIANT` on valid vectors.
- **Gradient guard:** the code comment must state "no training path touches
  this lens"; enforcement is structural (frozen weights are read-only `&[f32]`
  slices) — not a runtime check in this phase.
