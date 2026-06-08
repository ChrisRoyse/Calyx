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

`calyx-registry` has T01–T06 implemented: `FrozenLensContract`,
content-addressed `LensId`, finite/dim/norm guards, determinism probes,
algorithmic/TEI/local runtimes, and registry enforcement. Post-sweep #310 made
the boundary fail-closed: `Registry::register` and `register_with_spec` return
`CALYX_LENS_FROZEN_VIOLATION` and do not insert; successful callers must use
`register_frozen`, `register_frozen_with_spec`, or `register_frozen_with_probe`.

**aiwonder runtime endpoints:** `:8088` general GTE 768-d, `:8089` reranker,
`:8090` legal. `CALYX_HOME/.hf-cache`, `CALYX_HF_TOKEN` from env.

## Deliverables (file plan, each ≤500 lines)

| File | Responsibility |
|---|---|
| `crates/calyx-registry/src/frozen.rs` | frozen contract construction, LensId derivation, finite/norm checks, determinism probe |
| `crates/calyx-registry/src/lens.rs` | `register_frozen*` success paths; plain `register*` fail-closed without insertion |
| `crates/calyx-registry/src/runtime/algorithmic.rs` | exposes the deterministic contract used to derive each algorithmic `LensId` |

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

1. Call plain `Registry::register`; read `CALYX_LENS_FROZEN_VIOLATION` and
   confirm `Registry::contains(id) == false`.
2. Register a `TeiHttpLens`; swap its `weights_sha256` to a wrong value;
   attempt measure → `CALYX_LENS_FROZEN_VIOLATION` returned, no vector produced.
3. Register a lens declaring `SlotShape::Dense(128)`; runtime returns
   `Dense(768)` → `CALYX_LENS_DIM_MISMATCH`.
4. Register the same frozen contract in two `Registry` instances (simulating two
   vaults); `LensId` bytes are identical in both — read with
   `println!("{:x}", lens_id)` and confirm equality.

Readback: #310 captured the Stage 3 atomic FSV JSON at
`/home/croyse/calyx/data/fsv-issue310-registry-frozen-contract-20260608`.
The JSON records `plain_register_error=CALYX_LENS_FROZEN_VIOLATION` and
`plain_register_inserted=false`, plus frozen duplicate and runtime readbacks.

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
