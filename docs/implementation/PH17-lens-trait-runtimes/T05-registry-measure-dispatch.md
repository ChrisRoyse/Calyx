# PH17 · T05 — Registry.measure / measure_batch dispatch

| Field | Value |
|---|---|
| **Phase** | PH17 — Lens trait + algorithmic + tei-http runtimes |
| **Stage** | S3 — Registry / Lenses |
| **Crate** | `calyx-registry` |
| **Files** | `crates/calyx-registry/src/lib.rs` (≤500) |
| **Depends on** | T02, T03 (this phase) |
| **Axioms** | A4, A6 |
| **PRD** | `dbprdplans/05 §2`, `13_STAGE3_REGISTRY.md §PH17` |

## Goal

Wire `Registry.measure(lens_id, input)` and `Registry.measure_batch(lens_id,
inputs)` as the single uniform call site that dispatches to whichever runtime
backs the registered lens. This is the primary public API promised by the
stage objective.

## Build (checklist of concrete, code-level steps)

- [x] `Registry::measure(&self, lens_id: LensId, input: &Input) -> Result<SlotVector>`:
  - look up `lens_id` in `self.lenses`; if absent → `CALYX_REGISTRY_LENS_NOT_FOUND`.
  - call the stored `Box<dyn Lens>::measure(input)`.
  - return the `SlotVector` unchanged (frozen contract enforced in PH18).
- [x] `Registry::measure_batch(&self, lens_id: LensId, inputs: &[Input]) -> Result<Vec<SlotVector>>`:
  - look up lens; if absent → `CALYX_REGISTRY_LENS_NOT_FOUND`.
  - call `Box<dyn Lens>::measure_batch(inputs)`.
  - assert returned vec length equals inputs length; if mismatch →
    `CALYX_REGISTRY_RUNTIME_UNAVAILABLE` with remediation.
- [x] `Registry::health(&self, lens_id: LensId) -> Result<LensHealth>`:
  - returns stored `LensHealth`; `CALYX_REGISTRY_LENS_NOT_FOUND` if absent.
- [x] `Registry::panel_version(&self) -> u32`: monotone counter; increments on
  every `register` (stubs out slot allocation until PH20).
- [x] Ensure public re-exports from `lib.rs`:
  `pub use lens::{LensSpec, LensRuntime, LensHealth, LensCost, NormPolicy};`
  `pub use calyx_core::{Lens, Input, SlotShape, SlotVector, LensId, Modality};`

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: register an `AlgorithmicLens`; call `Registry::measure` with a
  known input; assert returned `SlotVector` dim matches `LensSpec.output`.
- [x] unit: `measure_batch` with 5 inputs → returned vec length is 5.
- [x] proptest: for any valid `Input`, `measure` result dim == `spec.output`
  declared dim (property holds for algorithmic lens with seeded inputs).
- [x] edge (≥3): (1) `measure` with unregistered `LensId` →
  `CALYX_REGISTRY_LENS_NOT_FOUND`; (2) `measure_batch` with empty slice →
  `Ok(vec![])` (no error); (3) `health` on unknown id →
  `CALYX_REGISTRY_LENS_NOT_FOUND`.
- [x] fail-closed: unregistered lens → exact `CALYX_REGISTRY_LENS_NOT_FOUND`.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `cargo test -p calyx-registry dispatch` test output on aiwonder
- **Readback:** `cargo test -p calyx-registry -- --nocapture 2>&1 | grep -E 'dispatch|measure|PASS'`
- **Prove:** log shows `Registry::measure dispatched to AlgorithmicLens → Dense(4)`;
  an identical second call returns identical bytes; attached to PH17 GitHub issue

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] FSV evidence (readback output / screenshot) attached to the PH17 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
