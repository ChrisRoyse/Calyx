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
#648 hardens T01 so `alpha` is no longer metadata-only: threshold candidates
must pass a binomial one-sided false-accept confidence check before calibration
accepts them. If the sample cannot certify the requested confidence, Ward uses
the strictest observed-bad threshold (`max_bad + 1 ULP`) instead of reporting a
looser alpha-insensitive tau. FSV roots:
`/home/croyse/calyx/data/fsv-issue648-alpha-bound-20260610` and
`/home/croyse/calyx/data/fsv-issue648-real-injection-20260610`.
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
#356 makes Sextant multi-slot InRegionOnly guarding slot-aware through
`Query.guard_vectors`, with FSV evidence at
`/home/croyse/calyx/data/fsv-issue356-sextant-multislot-guard-20260609-cfea3ac`.
#359 supplements #356 by reading back the actual query `guard_vectors` bytes,
candidate slot-vector bytes, and missing/sparse vector error edges at
`/home/croyse/calyx/data/fsv-issue359-sextant-guard-vector-readback-20260609-cf8d4b3`.
T07 (#279) is implemented and FSV-signed-off at
`/home/croyse/calyx/data/fsv-issue279-ward-ledger-provenance-20260609-55fc1da`.
It adds `calibrate_with_ledger()` and `guard_with_ledger()` wrappers that append
Ledger `EntryKind::Guard` rows for calibration and guard verdicts, then read
those rows back through PH36 audit/provenance while preserving the #349
quarantine contract.

## Deliverables (file plan, each ≤500 lines)

| File | Responsibility |
|---|---|
| `src/calibrate.rs` | conformal τ calibration per slot; alpha-sensitive binomial false-accept confidence check; empirical FAR is measured with Ward's `cos >= tau` predicate; slot-kind FAR caps; `CalibrationMeta` with `corpus_hash`, `estimator`, profile-summary `far`/`frr`, `confidence`, `ts`, and per-slot FAR/FRR in `per_slot`; provisional errors for invalid/insufficient calibration |
| `src/novelty.rs` | `NoveltyHandler`: route FAIL to `NewRegion` / `Quarantine` / `RejectClosed`; write novel constellation to the PH09-backed Aster vault CF |
| `src/drift.rs` | `DriftMonitor`: track rolling rejection/OOD rate per slot; fire Anneal hook when runtime rejection rate creeps above that slot's calibrated FAR bound; `guard_health()` exposes rejection rate, per-slot calibrated FAR bounds, FRR, drift flag, and last calibration timestamp |
| `src/lib.rs` | wire new modules; re-export `calibrate`, `novelty`, `drift` |
| `tests/calibrate_unit.rs` | deterministic calibration tests and manual aiwonder FSV fixture |
| `tests/novelty_handler.rs` | deterministic novelty routing tests and manual aiwonder FSV fixture |
| `tests/drift_monitor.rs` | deterministic drift-window/hook tests and manual aiwonder FSV fixture |
| `tests/ph38_injection_fsv.rs` | real injection corpus block-rate FSV and valid-novelty file-backed readback |
| `calyx-sextant/src/query.rs` | `Query.guard_vectors` slot-aware produced vectors for multi-slot guards |
| `calyx-sextant/src/guarded.rs` | PH38 T06 InRegionOnly candidate filtering, dropped-hit readback, and #356 multi-slot fail-closed guard vector handling |
| `calyx-sextant/tests/guarded_search.rs` | PH38 T06 deterministic + manual aiwonder FSV fixture, including #356 multi-slot guard-vector readback |
| `calyx-sextant/tests/guarded_multislot_readback.rs` | #359 supplemental FSV for query guard-vector bytes, candidate slot-vector bytes, and missing/sparse guard-vector fail-closed edges |

## Tasks (atomic — all must pass for the phase to be DONE)

| Card | Title | Depends |
|---|---|---|
| T01 | Conformal τ calibration per slot — ROC + quantile | DONE / FSV #264 |
| T02 | `provisional` flag + `CALYX_GUARD_PROVISIONAL` high-stakes refuse | DONE / FSV #265 |
| T03 | `NoveltyHandler` — `NewRegion` / `Quarantine` / `RejectClosed` routing | DONE / FSV #266 |
| T04 | `DriftMonitor` + Anneal hook + `guard_health()` | DONE / FSV #267 |
| T05 | FSV: injection corpus blocked >=99% at calibrated FAR + valid-novelty -> new region | DONE / FSV #268 |
| T06 | Sextant `QueryGuard::InRegionOnly(GuardProfile)` filters hits to trusted regions | DONE / FSV #276 |
| T07 | Ledger provenance wiring: calibration + guard verdicts as `kind=Guard` | DONE / FSV #279 |

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

**Alpha-sensitive calibration bound:** #648 proves strict alpha changes tau
when the sample supports the confidence check. Readback
`alpha-confidence-bound.json` shows bad count `1000`, target FAR `0.05`,
strict alpha `0.01` -> tau `0.5895001292228699` / FAR `0.03400000184774399`,
loose alpha `0.20` -> tau `0.5868000984191895` / FAR
`0.0430000014603138`, distinct corpus hashes, and both `strict_tau_gt_loose_tau`
and `strict_far_lt_loose_far` true. The real injection-corpus readback at
`/home/croyse/calyx/data/fsv-issue648-real-injection-20260610` persists
`alpha=0.05`, `confidence=0.95`, tau `0.7752314805984497`,
`calibration_far=0.0`, held-out block rate `1.0`, and a verified SHA-256
manifest.

**High-stakes slot provenance:** #649 proves high-stakes guards require every
required slot to carry both explicit tau and `CalibrationMeta.per_slot`
provenance. The guard FSV root
`/home/croyse/calyx/data/fsv-issue649-guard-provisional-20260610` reads back a
calibrated high-stakes pass, a missing-tau refusal, and a profile-level-only
calibration refusal, all with a verified SHA-256 manifest. The Ledger FSV root
`/home/croyse/calyx/data/fsv-issue649-ledger-provenance-20260610` reads physical
calibration/verdict rows at seqs `[0,1]`, includes the row bytes, and proves the
refused profile-level-only call appends no unprovenanced Guard row.

**Inert guard profiles:** #650 proves Ward and trusted Sextant surfaces reject
runtime-inert profiles before pass/OOD selection. Empty required-slot profiles
and `KofN { k: 0 }` return `CALYX_GUARD_INERT_PROFILE`; the Ward Ledger fixture
proves no Guard row is appended for an inert profile, and the Sextant fixture
proves non-inert uncalibrated guarded search remains explicitly
`provisional=true`.

**GuardHealth serde compatibility:** #358 proves pre-#354 `GuardHealth` JSON
without `per_slot_calibrated_far_bound` deserializes successfully, defaults that
map to empty, and reserializes with the new field present. Evidence root:
`/home/croyse/calyx/data/fsv-issue358-guard-health-serde-20260609-b298497`.

**Drift hook retry:** #355 proves a full hook channel records one dropped event,
keeps the slot in drift, and retries notification after recovery. Slot 3 is
absent before retry and present after retry. Evidence root:
`/home/croyse/calyx/data/fsv-issue355-drift-retry-20260609-bd544a5`.

**Sextant multi-slot guard vectors:** #356 proves multi-slot InRegionOnly no
longer clones one dense query vector into every required slot. Queries with
`Query.guard_vectors` use each required slot's own vector; the readback keeps the
two-slot survivor `04040404040404040404040404040404`, drops the style-slot
mismatch `05050505050505050505050505050505`, and returns
`CALYX_SEXTANT_VECTOR_SHAPE` when a multi-slot guarded query omits
`guard_vectors`. Evidence root:
`/home/croyse/calyx/data/fsv-issue356-sextant-multislot-guard-20260609-cfea3ac`.
#359 adds direct byte readback of `guard-query.json` and
`candidate-slot-readback.json`: query slot 8 is dense `[1.0, 0.0]`, query slot 9
is dense `[0.0, 1.0, 0.0]`, candidate `0404...` has the matching slot 9 vector,
candidate `0505...` has `[1.0, 0.0, 0.0]`, and missing/sparse guard-vector edges
both return `CALYX_SEXTANT_VECTOR_SHAPE`. Evidence root:
`/home/croyse/calyx/data/fsv-issue359-sextant-guard-vector-readback-20260609-cf8d4b3`.

**Guard provenance:** #279 writes calibration and guard verdict entries to the
real Ledger and reads them back via PH36 audit/provenance before PH38 exit. This
is signed off at
`/home/croyse/calyx/data/fsv-issue279-ward-ledger-provenance-20260609-55fc1da`:
the FSV after-read lists physical `.ledger` rows `0`, `1`, and `2`, with row
`0` tagged `ward_calibration_v1`, row `2` tagged `ward_guard_verdict_v1`,
`audit(kind=Guard)` returning `[0,2]`, `get_provenance(cx1)` returning `[2]`,
and a matching quarantined Guard row failing closed with
`CALYX_LEDGER_CHAIN_BROKEN`.

## Risks / landmines

- Conformal calibration requires an `n ≥ 50` held-out calibration set (mirrors
  PH28's quorum rule); below quorum `calibrate()` must return `Err` with
  `CALYX_GUARD_PROVISIONAL` rather than an uncalibrated τ — fail closed.
- The merged profile-level calibration FAR/FRR are summaries; callers that need
  slot-specific health or drift comparison must read `CalibrationMeta.per_slot`
  and `GuardHealth.per_slot_calibrated_far_bound`. High-stakes guard calls
  require per-required-slot tau plus `CalibrationMeta.per_slot` provenance and
  fail closed with `CALYX_GUARD_PROVISIONAL` if either is absent.
- Empty required-slot profiles and `KofN { k: 0 }` are runtime-inert and must
  fail closed with `CALYX_GUARD_INERT_PROFILE` before high-stakes provenance,
  trusted-query, Ledger, or Sextant guarded-search verdict paths.
- The injection corpus on aiwonder must be a real set (aiwonder at
  `/home/croyse/calyx/data/injection_corpus/`); synthetic random vectors do
  not satisfy the FSV gate.
- `ts` in `CalibrationMeta` must come from the `Clock` trait — never
  `SystemTime::now()` in logic paths.
- Drift monitor must not double-fire if Anneal hook is slow; use a channel
  with bounded capacity and drop on overflow (backpressure).
