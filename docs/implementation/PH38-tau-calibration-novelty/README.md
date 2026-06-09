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
receives a callback on each rejection-rate drift event while comparing against
the calibrated FAR bound. Default cold-start τ ≈ 0.7 but the calibrated value
governs.
The runtime drift metric is rejection/OOD rate; `CalibrationMeta.far` remains
the profile-level calibrated false-accept-rate summary, while
`CalibrationMeta.per_slot` preserves each slot's own FAR/FRR bounds.

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
T06 (#276) is implemented and FSV-signed-off at
`/home/croyse/calyx/data/fsv-issue276-ph38-t06-20260609-c0b5d7f`.
#350 hardens T03 by failing closed when the supplied `GuardProfile.guard_id`
does not match `GuardVerdict.guard_id`, before any novelty sink write. That FSV
is signed off at
`/home/croyse/calyx/data/fsv-issue350-ph38-guard-id-mismatch-20260609-a1fca2f`.
#353 also re-exports the stable novelty error constants from the `calyx-ward`
crate root for public callers.
#357 normalizes Ward calibration, novelty, and `guard_health.last_calibrated`
timestamps to Unix milliseconds and is FSV-signed-off at
`/home/croyse/calyx/data/fsv-issue357-ph38-timestamp-units-20260609-6e3ff73`.
#351 renames runtime drift health/event surfaces to rejection/OOD rate while
preserving the calibrated FAR bound and is FSV-signed-off at
`/home/croyse/calyx/data/fsv-issue351-ph38-rejection-rate-20260609-c6a2ccc`.
#352 makes the injection FSV report held-out `test` split block rate separately
from train-split calibration FAR and is FSV-signed-off at
`/home/croyse/calyx/data/fsv-issue352-ph38-heldout-injection-20260609-210d995`.
#354 preserves per-slot calibration FAR/FRR through `CalibrationMeta.per_slot`,
`guard_health().per_slot_calibrated_far_bound`, and drift hook comparisons, with
FSV evidence at
`/home/croyse/calyx/data/fsv-issue354-ph38-per-slot-calibration-20260609-f672547`.
#358 adds backwards-compatible serde defaulting for legacy `GuardHealth` JSON
without `per_slot_calibrated_far_bound`, with FSV evidence at
`/home/croyse/calyx/data/fsv-issue358-guard-health-serde-20260609-b298497`.
#355 adds retry semantics after bounded Anneal hook backpressure, with FSV
evidence at `/home/croyse/calyx/data/fsv-issue355-drift-retry-20260609-bd544a5`.
Post-T06 hardening remains tracked in #356 (Sextant multi-slot query guarding).
T07 (#279) remains open for Ledger `kind=Guard` provenance before PH38 can be
treated as fully closed.

## Deliverables (file plan, each ≤500 lines)

| File | Responsibility |
|---|---|
| `src/calibrate.rs` | conformal τ calibration per slot; empirical FAR is measured with Ward's `cos >= tau` predicate; slot-kind FAR caps; `CalibrationMeta` with `corpus_hash`, `estimator`, profile-summary `far`/`frr`, `confidence`, `ts`, and per-slot FAR/FRR in `per_slot`; provisional errors for invalid/insufficient calibration |
| `src/novelty.rs` | `NoveltyHandler`: route FAIL to `NewRegion` / `Quarantine` / `RejectClosed`; write novel constellation to the PH09-backed Aster vault CF |
| `src/drift.rs` | `DriftMonitor`: track rolling rejection/OOD rate per slot; fire Anneal hook when runtime rejection rate creeps above that slot's calibrated FAR bound; `guard_health()` exposes rejection rate, per-slot calibrated FAR bounds, FRR, drift flag, and last calibration timestamp |
| `src/lib.rs` | wire new modules; re-export `calibrate`, `novelty`, `drift` |
| `tests/calibrate_unit.rs` | deterministic calibration tests and manual aiwonder FSV fixture |
| `tests/novelty_handler.rs` | deterministic novelty routing tests and manual aiwonder FSV fixture |
| `tests/drift_monitor.rs` | deterministic drift-window/hook tests and manual aiwonder FSV fixture |
| `tests/ph38_injection_fsv.rs` | real injection corpus block-rate FSV and valid-novelty file-backed readback |
| `calyx-sextant/src/guarded.rs` | PH38 T06 InRegionOnly candidate filtering and dropped-hit readback |
| `calyx-sextant/tests/guarded_search.rs` | PH38 T06 deterministic + manual aiwonder FSV fixture |

## Tasks (atomic — all must pass for the phase to be DONE)

| Card | Title | Depends |
|---|---|---|
| T01 | Conformal τ calibration per slot — ROC + quantile | DONE / FSV #264 |
| T02 | `provisional` flag + `CALYX_GUARD_PROVISIONAL` high-stakes refuse | DONE / FSV #265 |
| T03 | `NoveltyHandler` — `NewRegion` / `Quarantine` / `RejectClosed` routing | DONE / FSV #266 |
| T04 | `DriftMonitor` + Anneal hook + `guard_health()` | DONE / FSV #267 |
| T05 | FSV: injection corpus blocked >=99% at calibrated FAR + valid-novelty -> new region | DONE / FSV #268 |
| T06 | Sextant `QueryGuard::InRegionOnly(GuardProfile)` filters hits to trusted regions | DONE / FSV #276 |
| T07 | Ledger provenance wiring: calibration + guard verdicts as `kind=Guard` | OPEN #279 |

## FSV exit gate (the phase is DONE only when this is byte-proven on aiwonder)

**Injection corpus blocked >=99% at the calibrated FAR:** signed off in #268 on
aiwonder with `block_rate=0.99239546` over 263 prompt-injection rows from the
pinned `/home/croyse/calyx/data/injection_corpus` corpus. **Valid-novelty -> new
region:** the FSV fixture writes a file-backed novelty row and reads it back as
`AwaitingGrounding`. Evidence root:
`/home/croyse/calyx/data/fsv-issue268-ph38-t05-20260609-ff20d0a`.

**Sextant guarded search:** #276 must prove a before/after search hit set where
an OOD candidate is excluded, surviving hits carry the Ward verdict, and dropped
hits are readable from the guarded-search report/explain payload. Evidence root:
`/home/croyse/calyx/data/fsv-issue276-ph38-t06-20260609-c0b5d7f`.

**Novelty guard-id integrity:** #350 proves a mismatched `profile.guard_id` /
`verdict.guard_id` returns `CALYX_GUARD_ID_MISMATCH` and leaves the novelty sink
empty, while the same fixture re-reads the normal `NewRegion`, `Quarantine`,
and `RejectClosed` records. Evidence root:
`/home/croyse/calyx/data/fsv-issue350-ph38-guard-id-mismatch-20260609-a1fca2f`.

**Timestamp units:** #357 proves `CalibrationMeta.ts`, `NoveltyRecord.ts`, and
`guard_health.last_calibrated` all use the same injected Unix millisecond clock
value, with zero/max/overflow timestamp edge cases read back from JSON. Evidence
root:
`/home/croyse/calyx/data/fsv-issue357-ph38-timestamp-units-20260609-6e3ff73`.

**Drift metric semantics:** #351 proves `guard_health()` and drift hook event
readback report runtime `rejection_rate`, while `CalibrationMeta.far` remains a
calibrated false-accept-rate bound. Evidence root:
`/home/croyse/calyx/data/fsv-issue351-ph38-rejection-rate-20260609-c6a2ccc`.

**Held-out injection split:** #352 proves PH38 T05 calibration uses the
`train` split (`343` benign, `203` injection) and reports held-out `test`
injection block rate separately (`60/60` blocked, `block_rate=1.0`). Evidence
root:
`/home/croyse/calyx/data/fsv-issue352-ph38-heldout-injection-20260609-210d995`.

**Per-slot calibration health:** #354 proves `CalibrationMeta.per_slot` preserves
slot 1 FAR `0.01` / FRR `1.0` and slot 2 FAR `0.05` / FRR `0.0`; `guard_health`
reads those same per-slot FAR/FRR values; the drift hook event fires for slot 1
using the slot 1 FAR bound. Evidence root:
`/home/croyse/calyx/data/fsv-issue354-ph38-per-slot-calibration-20260609-f672547`.

**GuardHealth serde compatibility:** #358 proves pre-#354 `GuardHealth` JSON
without `per_slot_calibrated_far_bound` deserializes successfully, defaults that
map to empty, and reserializes with the new field present. Evidence root:
`/home/croyse/calyx/data/fsv-issue358-guard-health-serde-20260609-b298497`.

**Drift hook retry:** #355 proves a full hook channel records one dropped event,
keeps the slot in drift, and retries notification after recovery. Slot 3 is
absent before retry and present after retry. Evidence root:
`/home/croyse/calyx/data/fsv-issue355-drift-retry-20260609-bd544a5`.

**Guard provenance:** #279 must write calibration and guard verdict entries to
the real Ledger and read them back via PH36 audit/provenance before PH38 exit.

## Risks / landmines

- Conformal calibration requires an `n ≥ 50` held-out calibration set (mirrors
  PH28's quorum rule); below quorum `calibrate()` must return `Err` with
  `CALYX_GUARD_PROVISIONAL` rather than an uncalibrated τ — fail closed.
- The merged profile-level calibration FAR/FRR are summaries; callers that need
  slot-specific health or drift comparison must read `CalibrationMeta.per_slot`
  and `GuardHealth.per_slot_calibrated_far_bound`.
- The injection corpus on aiwonder must be a real set (aiwonder at
  `/home/croyse/calyx/data/injection_corpus/`); synthetic random vectors do
  not satisfy the FSV gate.
- `ts` in `CalibrationMeta` must come from the `Clock` trait — never
  `SystemTime::now()` in logic paths.
- Drift monitor must not double-fire if Anneal hook is slow; use a channel
  with bounded capacity and drop on overflow (backpressure).
