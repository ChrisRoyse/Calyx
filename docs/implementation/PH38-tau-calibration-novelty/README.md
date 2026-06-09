# PH38 — τ Calibration (Conformal) + Novelty → New Region

**Stage:** S8 — Ward Gτ Guard  ·  **Crate:** `calyx-ward`  ·
**PRD roadmap:** P6  ·  **Axioms:** A2, A12

## Objective

Calibrate the per-slot threshold `τ` against grounded outcomes using conformal
prediction: bound the false-accept rate (FAR) at a chosen confidence level
`1 − α` per slot. Identity slots are calibrated strict; stylistic slots loose.
An uncalibrated `τ` is tagged `provisional` and high-stakes domains must refuse
(fail closed). A FAIL under a calibrated guard opens a new safe region
(`NewRegion`) rather than silently accepting; the drift monitor hook (Anneal)
receives a callback on each FAR-creep event. Default cold-start τ ≈ 0.7 but
the calibrated value governs.

## Dependencies

- **Phases:** PH37 (Gτ gate + `GuardProfile`), PH28 (grounded outcomes —
  `AnchoredSet` with known-good / known-bad label annotations)
- **Provides for:** PH39 (identity-locked generation uses calibrated profiles),
  PH41 (TCT dedup uses calibrated τ), PH48 (Anneal drift-recalibration hook),
  PH71 (Leapable vault swap uses `CALYX_GUARD_PROVISIONAL` signal)

## Current state (build off what exists)

`calyx-ward` is active, not a stub: PH37 T01/T02 (#258/#259) shipped the
profile, verdict, and error surfaces, and PH37 T03 (#260) adds the first
`guard()` math slice. PH28 (KSG MI / grounded outcomes) is not yet built, so
`calibrate.rs` stubs the `AnchoredSet` input type and integrates when PH28
lands. All calibration math is self-contained (conformal quantile over a score
array).

## Deliverables (file plan, each ≤500 lines)

| File | Responsibility |
|---|---|
| `src/calibrate.rs` | conformal τ calibration per slot; bound FAR at confidence `1−α`; `CalibrationProvenance` with `corpus_hash`, `estimator`, `far`, `frr`, `confidence`, `ts`; `provisional` flag |
| `src/novelty.rs` | `NoveltyHandler`: route FAIL to `NewRegion` / `Quarantine` / `RejectClosed`; write novel constellation to vault (stub CF until PH09 live) |
| `src/drift.rs` | `DriftMonitor`: track rolling FAR per slot; fire Anneal hook when FAR creeps above calibrated bound; `guard_health()` |
| `src/lib.rs` | wire new modules; re-export `calibrate`, `novelty`, `drift` |
| `tests/calibrate_unit.rs` | deterministic calibration + novelty + drift tests |

## Tasks (atomic — all must pass for the phase to be DONE)

| Card | Title | Depends |
|---|---|---|
| T01 | Conformal τ calibration per slot — ROC + quantile | — |
| T02 | `provisional` flag + `CALYX_GUARD_PROVISIONAL` high-stakes refuse | T01 |
| T03 | `NoveltyHandler` — `NewRegion` / `Quarantine` / `RejectClosed` routing | T01 |
| T04 | `DriftMonitor` + Anneal hook + `guard_health()` | T03 |
| T05 | FSV: injection corpus blocked ≥99% at calibrated FAR + valid-novelty → new region | T04 |

## FSV exit gate (the phase is DONE only when this is byte-proven on aiwonder)

**Injection corpus blocked >=99% at the calibrated FAR:** on aiwonder, run the
real prompt-injection test set through `guard()` with the calibrated profile;
write the block-rate and per-slot verdict summary to durable JSON and assert
`block_rate >= 0.99`. **Valid-novelty -> new region:** present a vector outside
all existing tau balls; assert `NoveltyAction::NewRegion` fires and the novel
constellation record is written to the vault CF. Read both artifacts back with
`xxd` or `calyx readback`, attach hashes, and treat stdout only as a captured
log.

## Risks / landmines

- Conformal calibration requires an `n ≥ 50` held-out calibration set (mirrors
  PH28's quorum rule); below quorum `calibrate()` must return `Err` with
  `CALYX_GUARD_PROVISIONAL` rather than an uncalibrated τ — fail closed.
- The injection corpus on aiwonder must be a real set (aiwonder at
  `/home/croyse/calyx/data/injection_corpus/`); synthetic random vectors do
  not satisfy the FSV gate.
- `ts` in `CalibrationMeta` must come from the `Clock` trait — never
  `SystemTime::now()` in logic paths.
- Drift monitor must not double-fire if Anneal hook is slow; use a channel
  with bounded capacity and drop on overflow (backpressure).
