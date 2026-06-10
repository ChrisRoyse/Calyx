# PH37 — Gτ Guard Math + GuardProfile

**Stage:** S8 — Ward Gτ Guard  ·  **Crate:** `calyx-ward`  ·
**PRD roadmap:** P6  ·  **Axioms:** A3, A12

## Objective

Implement the per-slot cosine gate `Gτ` with all-required (or KofN) pass logic.
For every produced vector, Ward measures `cos(produced_k, matched_k) ≥ τ_k` on
each required slot and emits a structured per-slot verdict `(cos, tau, pass)`.
No flattened-vector path exists — the no-flatten rule (A3) is the only path.
An output that passes the slot average but fails any single required slot is
unconditionally rejected.

## Dependencies

- **Phases:** PH22 (slots/lenses — `SlotId`, panel structure, per-slot vectors),
  PH13 (Forge cosine — CUDA sm_120 + CPU SIMD `cos(a,b)` with bit-parity ≤ 1e-3)
- **Provides for:** PH38 (τ calibration builds on this gate), PH39 (identity-locked
  generation calls `guard()`), PH41 (TCT dedup reuses the cosine gate)

## Current state (build off what exists)

PH37 T01 (#258) is implemented in `calyx-ward::profile`: `GuardId`,
`GuardPolicy`, `NoveltyAction`, `CalibrationMeta`, and `GuardProfile` are wired
through `lib.rs`, use `calyx_core::SlotId`, and serde round-trip
deterministically. aiwonder FSV wrote and read back JSON artifacts under
`/home/croyse/calyx/data/fsv-issue258-ph37-t01-20260609-tsus`.
PH37 T02 (#259) adds `SlotVerdict`, `GuardVerdict`, and `WardError` with typed
fail-closed codes in `calyx-ward::{verdict,error}`. PH37 T03 (#260) adds
`calyx-ward::guard` with `ProducedSlots`, `MatchedSlots`, `DEFAULT_TAU`, and
the `AllRequired` per-slot Forge cosine gate. PH37 T04 (#261) adds `KofN`
policy handling and `guard_result()` OOD wrapping. No-average enforcement and
PH37 T05 (#262) adds no-average/no-flatten source enforcement plus
average-pass/slot-fail rejection readback. PH37 T06 (#263) adds the phase FSV
readback harness and signs off the PH37 core path. PH37 T07 (#275) adds the
incoming-query `guard_query` OOD gate and is FSV-signed-off. PH37 T08 (#277)
adds Assay-derived required-slot selection from load-bearing `Slot.bits_about`
entries and is FSV-signed-off. PH37 T09 (#278) adds Lodestar-fed kernel-near
guard priority with source-marked verdicts and is FSV-signed-off. PH37 is
covered; PH38 conformal tau calibration T01 (#264) is also FSV-signed-off.
Post-sweep hardening #650 rejects runtime-inert guard profiles on Ward and
trusted Sextant surfaces: empty `required_slots` and `KofN { k: 0 }` now fail
closed with `CALYX_GUARD_INERT_PROFILE`.

Before #258, `calyx-ward` had only crate metadata. Ward depends on slots/lenses
(PH22) and Forge cosine (PH13); those dependency surfaces are already Stage 1-2
signed off, and #260 must call the actual Forge backend API rather than a
non-existent helper.

## Deliverables (file plan, each ≤500 lines)

| File | Responsibility |
|---|---|
| `src/guard.rs` | `cos(produced_k, matched_k) ≥ τ_k` gate; per-slot verdict; `AllRequired`/`KofN` policy; `CALYX_GUARD_OOD` |
| `src/query.rs` | incoming-query `guard_query` gate over trusted regions; `Pass`/`Ood { nearest_cx, gap }` verdict |
| `src/query.rs` | `guard_query_kernel_first` kernel-near priority and source-marked verdicts |
| `src/required.rs` | required-slot derivation from Assay `bits_about` using the inclusive 0.05-bit threshold; manual override |
| `src/profile.rs` | `GuardProfile` struct, `GuardPolicy` enum, `CalibrationMeta`, `NoveltyAction` enum, serde |
| `src/verdict.rs` | `GuardVerdict` (pass flag + `Vec<SlotVerdict { slot, cos, tau, pass }>`) |
| `src/lib.rs` | crate root; re-exports; module wiring |
| `src/error.rs` | `WardError` enum wrapping `CALYX_GUARD_OOD`, `CALYX_GUARD_PROVISIONAL` |
| `tests/guard_no_flatten.rs` | deterministic unit + property tests for the no-flatten gate |

## Tasks (atomic — all must pass for the phase to be DONE)

| Card | Title | Depends |
|---|---|---|
| T01 | `GuardProfile` struct + `GuardPolicy` + `NoveltyAction` + `CalibrationMeta` | — |
| T02 | `SlotVerdict` + `GuardVerdict` types + `WardError` catalog | T01 |
| T03 | `guard()` per-slot cosine gate — `AllRequired` policy | T02 |
| T04 | `guard()` `KofN` policy + `CALYX_GUARD_OOD` fail-closed | T03 |
| T05 | No-flatten enforcement + average-passing/slot-failing rejection | T04 |
| T06 | FSV harness — per-slot verdict readback + anti-flatten smoke test | T05 |
| T07 | `guard_query` incoming-query OOD gate | T06 |
| T08 | Required-slot set derived from Assay load-bearing bits | T07 |
| T09 | Kernel-near guard priority for Lodestar-fed queries | T08 |

## FSV exit gate (the phase is DONE only when this is byte-proven on aiwonder)

An output that passes the slot-level cosine average but fails one required slot
must be **rejected** with `CALYX_GUARD_OOD`. Prove by constructing a
`GuardProfile` with two required slots where slot-1 scores 0.95 (passes τ=0.7)
and slot-2 scores 0.50 (fails τ=0.7); call `guard()`; read the returned
`GuardVerdict::per_slot` bytes and confirm `pass=false` for slot-2 and
`overall_pass=false`. No flatten path must exist in the source (`grep -n flatten
src/guard.rs` must return empty).

## Risks / landmines

- Forge cosine is available through the `calyx-forge` backend API; do not name
  or implement a parallel Ward-only cosine helper unless it is only a thin
  wrapper over `Backend::cosine`.
- `SlotId` ordering across the `Map<SlotId,f32>` must be deterministic for
  bit-parity tests; use a `BTreeMap` internally.
- `KofN` with `k == 0` is not a trusted guard and must fail closed with
  `CALYX_GUARD_INERT_PROFILE`; `k` greater than the unique required-slot count
  must fail closed, not panic.
- Empty required-slot profiles are inert runtime profiles; serde may preserve
  them for compatibility, but guard execution must reject them before a pass or
  OOD verdict.
- Per-slot `(cos, tau, pass)` must be in the verdict even on overall PASS — the
  caller always gets full decomposition.
