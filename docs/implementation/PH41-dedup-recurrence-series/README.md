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
The `ingest` path exists in `crates/calyx-aster/src/vault.rs`; PH41 T02 now
builds the content-slot cosine gate on top of the persisted policy. The `Gτ`
guard (PH37) is in `calyx-ward`. `calyx-loom` exists from Stage 5
(cross-terms/agreement/abundance); PH41 should add a new `recurrence` module
under that crate rather than initialize the crate from scratch.

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
| T02 | Dedup engine: per-slot cosine gate (content-only, excl. E2/E3/E4) | T01 |
| T03 | Anchor-conflict guard (MUST NOT merge conflicting anchors) | T02 |
| T04 | `ingest_at(input, at: t)` → `New | DedupMerge{into, occurrence}` | T03 |
| T05 | Recurrence series store (one event, many `t_k` occurrences; bounded, A26) | T04 |
| T06 | Recurrence signature detector (content-agree + temporal-differ) | T05 |
| T07 | `dedup_audit` (per-slot cos, reversible, Ledger-logged) | T06 |
| T08 | FSV: near-but-distinct NOT merged; conflicting-anchor stays separate; recurring → series (reversible) | T07 |

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
