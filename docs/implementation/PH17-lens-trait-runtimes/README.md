# PH17 — Lens trait + algorithmic + tei-http runtimes

> **Status: DONE / FSV-signed-off.** `calyx-registry` now implements the
> uniform lens runtime layer, algorithmic and TEI-HTTP runtimes, and downstream
> PH18–PH22 surfaces. See `13_STAGE3_REGISTRY.md` for the current Stage 3
> readback root.

**Stage:** S3 — Registry / Lenses  ·  **Crate:** `calyx-registry`  ·
**PRD roadmap:** P2  ·  **Axioms:** A4, A6

## Objective

Establish a uniform `Registry.measure(lens_id, input)` call over multiple
runtimes so that every caller is insulated from how a vector is produced.
Ship the `algorithmic` (deterministic, no-NN) and `tei-http` runtimes first,
reusing the resident TEI services on aiwonder (:8088 general gte 768-d, :8089
reranker, :8090 legal). Neither runtime requires CUDA at this phase; Forge
math (PH12) is already available for normalisation helpers.

## Dependencies

- **Phases:** PH12 (Forge CPU backend — cosine/normalize), PH09 (Aster
  constellation CRUD — SlotId/panel_version exist)
- **Provides for:** PH18 (frozen contract sits on top of the runtime layer),
  PH23 (Sextant calls `Registry.measure` via the same path)

## Current state (build off what exists)

`calyx-registry` is implemented and no longer a stub. The `Lens` trait,
`Input`, `SlotShape`, `SlotVector`, `LensId`, and `Modality` types live in
`calyx-core`; the Registry runtime layer builds against those shared types and
does not redefine them.

**aiwonder runtime endpoints (build/test there only):**
- `:8088` — general GTE 768-d (`BAAI/bge-m3` or equivalent resident model)
- `:8089` — reranker
- `:8090` — legal domain embedder
- `CALYX_HOME/.hf-cache` — local HF model cache; `CALYX_HF_TOKEN` env var

## Deliverables (file plan, each ≤500 lines)

| File | Responsibility |
|---|---|
| `crates/calyx-registry/src/lib.rs` | `Registry` struct, `measure`/`measure_batch` dispatch, public re-exports |
| `crates/calyx-registry/src/lens.rs` | `LensSpec`, `LensRuntime` enum (all five variants declared), `LensHealth`, `LensCost`, `NormPolicy`, registration bookkeeping |
| `crates/calyx-registry/src/runtime/mod.rs` | runtime sub-module declarations |
| `crates/calyx-registry/src/runtime/algorithmic.rs` | `AlgorithmicLens`: scalar, one-hot, AST-style deterministic feature encoders |
| `crates/calyx-registry/src/runtime/tei_http.rs` | `TeiHttpLens`: HTTP client to resident TEI, batching, determinism probe |
| `crates/calyx-registry/src/error.rs` | registry-specific `CALYX_REGISTRY_*` error codes wired to `CalyxError` |
| `crates/calyx-registry/src/tests/` | integration + unit tests (seeded, deterministic) |

## Tasks (atomic — all must pass for the phase to be DONE)

| Card | Title | Depends |
|---|---|---|
| T01 | Registry struct + LensSpec + LensRuntime enum | — |
| T02 | AlgorithmicLens runtime | T01 |
| T03 | TeiHttpLens runtime + batching | T01 |
| T04 | Determinism probe (embed twice → identical) | T03 |
| T05 | Registry.measure / measure_batch dispatch | T02, T03 |
| T06 | Error catalog + fail-closed paths | T01 |

## FSV exit gate (the phase is DONE only when this is byte-proven on aiwonder)

1. Send the same UTF-8 text input to `:8088` **twice** via `TeiHttpLens.measure`
   → assert `v1 == v2` element-for-element (determinism probe).
2. Run an `AlgorithmicLens` on a known scalar input → print the result vector;
   re-run → assert bit-for-bit identical output.
3. `Registry.measure(lens_id, input)` dispatches correctly for both runtimes,
   returning a `SlotVector::Dense(768)` from TEI and the declared shape from
   algorithmic.

Readback: `cargo test -p calyx-registry -- --nocapture 2>&1 | grep FSV` on
aiwonder; embed output hex-dumped to the PH17 GitHub issue.

## Risks / landmines

- **TEI connection refused during CI:** tests that call `:8088` must be
  `#[ignore]` by default and run explicitly on aiwonder with
  `cargo test -- --include-ignored`; never fail the build when TEI is absent.
- **Never start a throwaway TEI process** (`05 §2` gotcha): always reuse the
  resident services; spawning a new TEI instance contends VRAM with them.
- **Batch ceiling:** TEI has a default max-batch-size; the client must chunk
  inputs and reassemble; test with both single and >32-input batches.
- **Unit-norm contract at this phase:** `TeiHttpLens` should assert unit-norm
  on the returned vector in debug builds; the hard enforcement lands in PH18.
