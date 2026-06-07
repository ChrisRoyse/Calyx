# PH40 — Temporal Fusion + AP-60 Post-Retrieval Boost

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

`calyx-sextant` is a 9-line stub (`crates/calyx-sextant/src/lib.rs`);
greenfield. PH24 (which lands in sextant) has not yet been implemented either —
PH40 depends on PH24 being done first. E2/E3/E4 lens math lands in
`calyx-registry` at PH22 (closed-form, deterministic, no trained weights).

## Deliverables (file plan, each ≤500 lines)

| File | Responsibility |
|---|---|
| `crates/calyx-sextant/src/temporal/mod.rs` | `TemporalPolicy`, `FusionWeights`, `TimeWindow`, `BoostConfig`; AP-60 invariant enforced |
| `crates/calyx-sextant/src/temporal/boost.rs` | `apply_temporal_boost(hits, policy, query_time, clock)` — post-retrieval reranker |
| `crates/calyx-sextant/src/temporal/window.rs` | `last_hours(n)` / `last_days(n)` constructors + window filter |
| `crates/calyx-sextant/src/temporal/causal_gate.rs` | causal-confidence gate (high-conf ×1.10, low ×0.85) |
| `crates/calyx-sextant/src/temporal/tests.rs` | deterministic unit + property tests for all boost/window logic |

## Tasks (atomic — all must pass for the phase to be DONE)

| Card | Title | Depends |
|---|---|---|
| T01 | TemporalPolicy + FusionWeights types | — |
| T02 | TimeWindow helpers (`last_hours`/`last_days`) | T01 |
| T03 | `apply_temporal_boost` post-retrieval reranker | T02 |
| T04 | Causal confidence gate (×1.10 / ×0.85) | T03 |
| T05 | AP-60 invariant enforcement + `temporal_search` integration | T04 |
| T06 | FSV: temporal-never-dominant + boost-reorder proof | T05 |

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
