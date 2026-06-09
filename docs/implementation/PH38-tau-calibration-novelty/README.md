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

`calyx-ward` is active, not a stub: PH37 T01-T09 (#258-#263, #275,
#277, #278) shipped the profile, verdict, error, AllRequired/KofN guard math,
no-flatten enforcement, PH37 readback harness, incoming-query OOD gating,
Assay-derived required slots, and Lodestar kernel-near priority. PH28 is
FSV-backed, so PH38 T01 (#264) accepts grounded known-good / known-bad cosine
score arrays today and can later receive those arrays directly from
`AnchoredSet` adapters without changing the calibration math. T01 is
implemented and FSV-signed-off at
`/home/croyse/calyx/data/fsv-issue264-ph38-t01-20260609-f95c817`. T02 (#265)
is implemented and FSV-signed-off at
`/home/croyse/calyx/data/fsv-issue265-ph38-t02-20260609-5c23db5`.
T03 (#266) is implemented and FSV-signed-off at
`/home/croyse/calyx/data/fsv-issue266-ph38-t03-20260609-fa0c263`.
T04 (#267) is implemented and FSV-signed-off at
`/home/croyse/calyx/data/fsv-issue267-ph38-t04-20260609-912b707`.
T05 (#268) is implemented and FSV-signed-off at
`/home/croyse/calyx/data/fsv-issue268-ph38-t05-20260609-ff20d0a`.

## Deliverables (file plan, each ≤500 lines)

| File | Responsibility |
|---|---|
| `src/calibrate.rs` | conformal τ calibration per slot; empirical FAR is measured with Ward's `cos >= tau` predicate; slot-kind FAR caps; `CalibrationMeta` with `corpus_hash`, `estimator`, `far`, `frr`, `confidence`, `ts`; provisional errors for invalid/insufficient calibration |
| `src/novelty.rs` | `NoveltyHandler`: route FAIL to `NewRegion` / `Quarantine` / `RejectClosed`; write novel constellation to the PH09-backed Aster vault CF |
| `src/drift.rs` | `DriftMonitor`: track rolling FAR per slot; fire Anneal hook when FAR creeps above calibrated bound; `guard_health()` |
| `src/lib.rs` | wire new modules; re-export `calibrate`, `novelty`, `drift` |
| `tests/calibrate_unit.rs` | deterministic calibration tests and manual aiwonder FSV fixture |
| `tests/novelty_handler.rs` | deterministic novelty routing tests and manual aiwonder FSV fixture |
| `tests/drift_monitor.rs` | deterministic drift-window/hook tests and manual aiwonder FSV fixture |
| `tests/ph38_injection_fsv.rs` | real injection corpus block-rate FSV and valid-novelty file-backed readback |

## Tasks (atomic — all must pass for the phase to be DONE)

| Card | Title | Depends |
|---|---|---|
| T01 | Conformal τ calibration per slot — ROC + quantile | DONE / FSV #264 |
| T02 | `provisional` flag + `CALYX_GUARD_PROVISIONAL` high-stakes refuse | DONE / FSV #265 |
| T03 | `NoveltyHandler` — `NewRegion` / `Quarantine` / `RejectClosed` routing | DONE / FSV #266 |
| T04 | `DriftMonitor` + Anneal hook + `guard_health()` | DONE / FSV #267 |
| T05 | FSV: injection corpus blocked >=99% at calibrated FAR + valid-novelty -> new region | DONE / FSV #268 |

## FSV exit gate (the phase is DONE only when this is byte-proven on aiwonder)

**Injection corpus blocked >=99% at the calibrated FAR:** signed off in #268 on
aiwonder with `block_rate=0.99239546` over 263 prompt-injection rows from the
pinned `/home/croyse/calyx/data/injection_corpus` corpus. **Valid-novelty -> new
region:** the FSV fixture writes a file-backed novelty row and reads it back as
`AwaitingGrounding`. Evidence root:
`/home/croyse/calyx/data/fsv-issue268-ph38-t05-20260609-ff20d0a`.

## Risks / landmines

- Conformal calibration requires an `n ≥ 50` held-out calibration set (mirrors
  PH28's quorum rule); below quorum `calibrate()` must return `Err` with
  `CALYX_GUARD_PROVISIONAL` rather than an uncalibrated τ — fail closed.
- The merged profile-level calibration FAR is the max across calibrated slots;
  identity-slot FAR is separately proven <=0.01 in T01 readback.
- The injection corpus on aiwonder must be a real set (aiwonder at
  `/home/croyse/calyx/data/injection_corpus/`); synthetic random vectors do
  not satisfy the FSV gate.
- `ts` in `CalibrationMeta` must come from the `Clock` trait — never
  `SystemTime::now()` in logic paths.
- Drift monitor must not double-fire if Anneal hook is slow; use a channel
  with bounded capacity and drop on overflow (backpressure).
