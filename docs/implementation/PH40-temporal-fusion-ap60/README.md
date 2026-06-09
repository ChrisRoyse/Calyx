# PH40 — Temporal Fusion + AP-60 Post-Retrieval Boost

> **Status: active Stage 9 work.** `calyx-sextant` (PH23–PH26) and Registry
> temporal lenses (PH22) are implemented and FSV-signed-off. PH40 adds temporal
> post-retrieval boost modules to the existing Sextant stack rather than
> starting from a stub.

**Stage:** S9 — Temporal & Dedup  ·  **Crate:** `calyx-sextant`  ·
**PRD roadmap:** A27  ·  **Axioms:** A27

## Objective

Wire E2/E3/E4 temporal lenses into the search pipeline as a post-retrieval boost
only — never dominant, never present during primary ANN retrieval — implementing
the AP-60 invariant verbatim from the Royse corpus. Fusion weighting is 50%
recency (E2) + 35% sequence (E4) + 15% periodic (E3), tunable per vault. The
causal gate multiplies high-confidence hits ×1.10 and low-confidence hits ×0.85.
Time-window helpers (`last_hours(n)` / `last_days(n)`) scope queries without
distorting in-window ranking.

## Dependencies

- **Phases:** PH24 (RRF/WeightedRRF fusion + provenance hits — provides the
  ranked result list that receives the boost), PH22 (E2/E3/E4 temporal lenses
  registered in the default panel)
- **Provides for:** PH42 (grounded recurrence wiring — Sextant AP-60 boost is
  one of the seven engine wirings), PH49 (Oracle consequence prediction uses
  temporal search)

## Current state (build off what exists)

`calyx-sextant` now contains the Stage 4 search stack (dense/sparse indexes,
fusion, provenance, freshness, planner/explain). PH40 depends on those existing
modules and should wire AP-60 as a post-retrieval stage. E2/E3/E4 lens math is
already in `calyx-registry` from PH22 (closed-form, deterministic, no trained
weights).

## Deliverables (file plan, each ≤500 lines)

| File | Responsibility |
|---|---|
| `crates/calyx-core/src/temporal.rs` | shared `TemporalPolicy`, `FusionWeights`, `DecayFunction`, `PeriodicOptions`, `SequenceOptions`, `BoostConfig`; AP-60 invariant enforced at serde/write/read boundaries |
| `crates/calyx-sextant/src/temporal/mod.rs` | Sextant-facing re-export and PH40 T01 deterministic tests |
| `crates/calyx-aster/tests/temporal_manifest_fsv.rs` | T01 durable vault manifest FSV readback |
| `crates/calyx-sextant/src/temporal/boost.rs` | `apply_temporal_boost(hits, policy, query_time, clock)` — post-retrieval reranker |
| `crates/calyx-sextant/src/temporal/window.rs` | `last_hours(n)` / `last_days(n)` constructors + window filter |
| `crates/calyx-sextant/src/temporal/causal_gate.rs` | causal-confidence gate (high-conf ×1.10, low ×0.85) |
| `crates/calyx-sextant/src/temporal/tests.rs` | deterministic unit + property tests for all boost/window logic |

## Tasks (atomic — all must pass for the phase to be DONE)

| Card | Title | Depends |
|---|---|---|
| T01 #373 | TemporalPolicy + FusionWeights types | — |
| T02 #374 | TimeWindow helpers (`last_hours`/`last_days`) | T01 |
| T03 #375 | `apply_temporal_boost` post-retrieval reranker | T02 |
| T04 #376 | Causal confidence gate (×1.10 / ×0.85) | T03 |
| T05 #377 | AP-60 invariant enforcement + `temporal_search` integration | T04 |
| T06 #378 | FSV: temporal-never-dominant + boost-reorder proof | T05 |

## Completed PH40 Evidence

- T01 #373 commit: `9ca0a93`
- aiwonder FSV root:
  `/home/croyse/calyx/data/fsv-issue373-temporal-policy-manifest-20260609-9ca0a93`
- Source of truth: Aster durable vault `CURRENT`, immutable
  `manifest-00000000000000000001.json`, and mirror `MANIFEST`; all contain
  `temporal_policy.never_dominant = true`.
- Edge proofs: invalid `never_dominant=false` leaves no `CURRENT` in the
  attempted vault and returns `CALYX_TEMPORAL_AP60_VIOLATION`; zero weights
  return `CALYX_TEMPORAL_WEIGHT_SUM`; invalid hour returns
  `CALYX_TEMPORAL_INVALID_PERIOD`.
- T02 #374 commit: `d872c7c`
- aiwonder FSV root:
  `/home/croyse/calyx/data/fsv-issue374-time-window-20260609-d872c7c`
- Source of truth: `temporal-window-input.json`,
  `temporal-window-readback.json`, and `BLAKE3SUMS.txt` under the FSV root.
  Readback keeps only hit IDs 01 and 03 for window `[992800, 1000000)`, proving
  the out-of-window hit 02 is absent and retained order is unchanged. Edge
  proofs cover empty input, all-window retention of missing timestamps, and
  `CALYX_TEMPORAL_INVALID_WINDOW` for zero, reversed, and overflow windows.

## FSV exit gate (the phase is DONE only when this is byte-proven on aiwonder)

A recent/periodic item that does NOT match a content lens must **not** surface in
results — temporal never dominant. Read the ranked result list before and after
`apply_temporal_boost` to confirm boost only reorders, never promotes a
content-miss. `temporal weight = 0.0` must be visible in the raw retrieval trace
(explain output). Both before/after ranked lists read via
`calyx readback temporal_search --explain` on aiwonder with an injected fixed
clock and a synthetic two-result set where the content-miss is the most recent
item.

## Risks / landmines

- **Clock injection gap:** `SystemTime::now()` must never appear in boost logic;
  all time comparisons go through the `Clock` trait. Audit every call site before
  merge.
- **E2 relative to query-time not ingest-time:** E2 age must be computed as
  `query_time − event_time`, not `now() − ingest_time`. Ingest timestamps are
  available on the `Hit`; query-time is passed explicitly.
- **Timezone-aware E3:** periodic scoring (hour-of-day / day-of-week) must apply
  a timezone offset before extracting hour/dow; UTC-naive comparison is a silent
  correctness bug.
- **Fusion weight sum:** recency (0.50) + sequence (0.35) + periodic (0.15) = 1.0
  exactly. Tunable per vault but must re-normalize; assert sum ≈ 1.0 at
  construction.
- **PH24 dependency:** do not start T03 until PH24's `Hit` type is stable; the
  boost operates on `Hit` structs returned by fusion.
