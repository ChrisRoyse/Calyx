# PH41 — DedupPolicy TctCosine + Recurrence Series + Signature

**Stage:** S9 — Temporal & Dedup  ·  **Crate:** `calyx-aster` / `calyx-loom`  ·
**PRD roadmap:** A28, A29  ·  **Axioms:** A28, A29, A3, A26

## Objective

Implement multi-content-slot TCT cosine-`Gτ` deduplication configurable at vault
creation. `DedupPolicy { Off | Exact | TctCosine { required_slots, tau, action } }`
governs how `ingest_at(input, at: t)` responds when a near-duplicate is detected:
collapse it, link it, or — when `action = RecurrenceSeries` — append a timestamped
occurrence to a recurrence series. The recurrence signature (all content slots
agree + temporal lenses differ) is the detector that fires automatically, routing
the same action at a new time into the series. Dedup operates on content slots
only (temporal lenses E2/E3/E4 are excluded). Constellations with conflicting
anchors MUST NOT be merged. All merges are reversible and Ledger-logged. The
`dedup_audit` function exposes per-slot cosines and the full merge history.

## Dependencies

- **Phases:** PH37 (`Gτ` guard math + `GuardProfile` — provides the per-slot
  cosine gate and calibrated `τ`), PH09 (constellation CRUD + idempotent ingest
  — provides the `ingest` entry point that this phase extends)
- **Provides for:** PH42 (grounded recurrence wiring — needs the recurrence
  series and occurrence count stored here), PH72 (streaming ingest + time-travel
  depend on the recurrence series)

## Current state (build off what exists)

`calyx-aster` has WAL, memtable, SSTable, column families, MVCC, constellation
CRUD, manifest, compaction, and PH41 T01 `DedupPolicy` manifest persistence in
place. #379 is FSV-signed-off at
`/home/croyse/calyx/data/fsv-issue379-dedup-policy-20260610-0083015`.
PH41 T02 #380 is FSV-signed-off at
`/home/croyse/calyx/data/fsv-issue380-dedup-validation-20260610-5af9a20`: the
bounded content-slot cosine gate now runs on top of the persisted policy and
prints byte readback through `calyx readback dedup-check`, including fail-closed
runtime validation for calibrated tau and constructor-bypassed empty
`required_slots`.
PH41 T03 #381 is FSV-signed-off at
`/home/croyse/calyx/data/fsv-issue381-anchor-conflict-20260610-00c0540`: the
dedup engine now checks shared anchors before cosine, returns `AnchorConflict`
for opposite `SpeakerMatch`, incompatible `StyleHold`, and exclusive-tag
conflicts, and writes reciprocal `dedup:contested_with:<CxId>` rows through the
durable `online` CF/WAL path. Exact/same-CxId anchor conflicts now fail closed
instead of matching through the exact/self path.
PH41 T04 #382 is FSV-signed-off at
`/home/croyse/calyx/data/fsv-issue382-ingest-at-20260610-1a0c560`: the Aster
`ingest_at` facade stores caller-provided event time in base rows, interim
recurrence Online CF rows, and Ledger payloads; exact duplicates write Ledger
without a second base row; anchor conflicts store a new candidate plus reciprocal
contested rows; invalid negative event time fails closed with no base/ledger
rows.
PH41 T05 #383 is FSV-signed-off at
`/home/croyse/calyx/data/fsv-issue383-recurrence-series-20260610-bacf9d2`: Aster
now owns the dedicated `recurrence` CF, Loom exposes `SeriesStore` as the
facade, recurrence ingests update base `recurrence.frequency` and recurrence
rows in one commit, CLI `readback recurrence-series` reads SST+WAL bytes, and
the FSV fixture proves happy-path 5 occurrences, empty series, max-count rollup,
oversized context fail-closed, and WAL append failure atomicity. The `Gτ` guard
(PH37) is in `calyx-ward`.
PH41 T06 #384 is FSV-signed-off at
`/home/croyse/calyx/data/fsv-issue384-recurrence-signature-20260610-8b0d0bb`:
the signature detector now distinguishes same-action/new-time recurrence from
same-time exact duplicate, routes valid signatures into recurrence occurrence
appends, fails closed on missing temporal signature slots, and records
`recurrence_signature`, `same_action`, and `new_time` in Ledger payloads.

## Deliverables (file plan, each ≤500 lines)

| File | Responsibility |
|---|---|
| `crates/calyx-aster/src/dedup/mod.rs` | `DedupPolicy`, `DedupAction`, `TctCosineConfig`, `DedupResult` types |
| `crates/calyx-aster/src/dedup/policy.rs` | Policy validation, anchor-conflict check, content-slot selection (excl. E2/E3/E4) |
| `crates/calyx-aster/src/dedup/engine.rs` | `check_dedup(new_cx, vault, policy) -> DedupDecision`; per-slot cosine, required-slots pass logic |
| `crates/calyx-aster/src/dedup/ingest_at.rs` | `ingest_at(vault, input, at: t) -> New(CxId) | DedupMerge{into, occurrence}` |
| `crates/calyx-loom/src/recurrence/mod.rs` | `RecurrenceSeries`, `Occurrence { t_k, context }`, bounded rollup/retention (A26) |
| `crates/calyx-loom/src/recurrence/series_store.rs` | CF-backed store: append occurrence, read series, cadence scalar |
| `crates/calyx-loom/src/recurrence/signature.rs` | Recurrence signature detector: content-slots-agree + temporal-slots-differ |
| `crates/calyx-aster/src/dedup/audit.rs` | `dedup_audit(vault, cx) -> DedupAuditReport { per_slot_cos, merges, reversible }` |
| `crates/calyx-aster/src/dedup/tests.rs` | All dedup FSV tests |
| `crates/calyx-loom/src/recurrence/tests.rs` | All recurrence series FSV tests |

## Tasks (atomic — all must pass for the phase to be DONE)

| Card | Title | Depends |
|---|---|---|
| T01 | `DedupPolicy` types + vault-creation config | DONE / FSV #379 |
| T02 | Dedup engine: per-slot cosine gate (content-only, excl. E2/E3/E4) | DONE / FSV #380 |
| T03 | Anchor-conflict guard (MUST NOT merge conflicting anchors) | DONE / FSV #381 |
| T04 | `ingest_at(input, at: t)` → `New | DedupMerge{into, occurrence}` | DONE / FSV #382 |
| T05 | Recurrence series store (one event, many `t_k` occurrences; bounded, A26) | DONE / FSV #383 |
| T06 | Recurrence signature detector (content-agree + temporal-differ) | DONE / FSV #384 |
| T07 | `dedup_audit` (per-slot cos, reversible, Ledger-logged) | T06 |
| T08 | FSV: near-but-distinct NOT merged; conflicting-anchor stays separate; recurring → series (reversible) | T07 |

## Tracked PH41 follow-ups

| Issue | Scope |
|---|---|
| #578 | Public `recurrence_series`, `periodic_fit`, and `periodic_recall` read APIs before PH41 exit |
| #617 | Durable/recovered `DedupPolicy` validation parity for temporal required-slot rejection |
| #620 | Recurrence rollup tombstone/physical reclaim integration |
| #621 | Concurrency-safe recurrence occurrence id allocation |
| #622 | Exact WAL write-failure code/injection proof beyond current storage-error fail-closed path |

## FSV exit gate (the phase is DONE only when this is byte-proven on aiwonder)

Three gates, all must pass:
1. **Near-but-distinct NOT merged:** ingest two constellations with content cosine
   just below calibrated τ → `New` returned for both → two separate CxIds exist in
   CF (`calyx readback cx-list`).
2. **Conflicting-anchor stays separate:** ingest two constellations with identical
   content slots but opposite `SpeakerMatch` anchors → `New` returned for second →
   two separate CxIds; `dedup_audit` shows anchor-conflict-blocked (`xxd` the CF).
3. **Recurring event → one event + time series (reversible):** ingest the same
   constellation 3× at different timestamps → one CxId + `RecurrenceSeries` with 3
   occurrences → `calyx readback recurrence-series <CxId>` shows 3 `t_k` entries →
   call `dedup_audit` → merge history reversible → apply reversal → original 3
   separate CxIds restored byte-for-byte.

## Risks / landmines

- **Temporal slots must be excluded from dedup agreement:** the `required_slots`
  in `TctCosineConfig` must never include `SlotId`s corresponding to E2/E3/E4. Add
  an explicit filter at policy construction time, not just convention.
- **Anchor-conflict check before cosine check:** check anchor compatibility first;
  if anchors conflict, skip cosine comparison and return `New`. Never compare
  cosines on a pair that will be refused anyway.
- **Bounded recurrence series (A26):** the series store must enforce a max
  occurrence count and a retention window; unbounded growth is a resource hazard.
  Rollup policy (collapse old occurrences into a summary) must be implemented.
- **Reversibility constraint:** every merge must write a Ledger entry containing
  enough information to reconstruct the original constellations. Test reversal
  byte-for-byte before merge.
- **`ingest_at` is the single ingest entry point:** all temporal ingests go through
  `ingest_at`; the existing `ingest` in PH09 becomes a thin wrapper with `at = now`.
