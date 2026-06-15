# PH40 — Temporal Fusion + AP-60 Post-Retrieval Boost

> **Status: implemented and FSV-signed-off through T06 plus #615/#616/#618/#619 hardening.**
> `calyx-sextant` (PH23-PH26) and Registry temporal lenses (PH22) are
> implemented and FSV-signed-off. PH40 added temporal post-retrieval boost
> modules to the existing Sextant stack rather than starting from a stub.
> Follow-ups #616, #618, and #619 are closed and FSV-backed for bounded
> overfetch before window filtering, negative fusion-weight validation, and
> public periodic scorer scope/query-time semantics.

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

## Current state (completed build on existing stack)

`calyx-sextant` now contains the Stage 4 search stack (dense/sparse indexes,
fusion, provenance, freshness, planner/explain). PH40 wires AP-60 as a
post-retrieval stage on those existing modules. E2/E3/E4 lens math is
already in `calyx-registry` from PH22 (closed-form, deterministic, no trained
weights).

## Deliverables (file plan, each ≤500 lines)

| File | Responsibility |
|---|---|
| `crates/calyx-core/src/temporal.rs` | shared `TemporalPolicy`, `FusionWeights`, `DecayFunction`, `PeriodicOptions`, `SequenceOptions`, `BoostConfig`; AP-60 invariant enforced at serde/write/read boundaries |
| `crates/calyx-sextant/src/temporal/mod.rs` | Sextant-facing re-export and PH40 T01 deterministic tests |
| `crates/calyx-aster/tests/temporal_manifest_fsv.rs` | T01 durable vault manifest FSV readback |
| `crates/calyx-sextant/src/temporal/boost.rs` | `apply_temporal_boost(hits, policy, query_time, tz_offset)` — content-relative post-retrieval reranker |
| `crates/calyx-sextant/src/temporal/window.rs` | `last_hours(n)` / `last_days(n)` constructors + window filter |
| `crates/calyx-sextant/src/temporal/causal_gate.rs` | causal-confidence gate (high-conf ×1.10, low ×0.85) |
| `crates/calyx-sextant/src/temporal/search.rs` | `temporal_search` AP-60 integration boundary, pre-boost ranking capture, non-positive final-surface filter, final truncate/renumber |
| `crates/calyx-sextant/src/temporal/tests.rs` | deterministic temporal-never-dominant, boost-reorder, query-time, timezone, and AP-60 violation proofs |
| `crates/calyx-sextant/tests/causal_gate_fsv.rs` | deterministic causal gate pipeline artifact readback |
| `crates/calyx-sextant/tests/temporal_search_fsv.rs` | deterministic temporal-search integration artifact readback |
| `crates/calyx-sextant/tests/temporal_never_dominant_fsv.rs` | deterministic #378 artifact readback for the phase exit gate |
| `crates/calyx-cli/src/temporal_readback.rs` | `calyx readback temporal_search --explain` FSV stdout surface |

## Tasks (atomic — all must pass for the phase to be DONE)

| Card | Title | Depends | Status |
|---|---|---|---|
| T01 #373 | TemporalPolicy + FusionWeights types | — | DONE / FSV |
| T02 #374 | TimeWindow helpers (`last_hours`/`last_days`) | T01 | DONE / FSV |
| T03 #375 | `apply_temporal_boost` post-retrieval reranker | T02 | DONE / FSV |
| T04 #376 | Causal confidence gate (x1.10 / x0.85) | T03 | DONE / FSV |
| T05 #377 | AP-60 invariant enforcement + `temporal_search` integration | T04 | DONE / FSV |
| T06 #378 | FSV: temporal-never-dominant + boost-reorder proof | T05 | DONE / FSV |

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
- Post-sweep hardening commit: `a54dcc1`
- aiwonder FSV root:
  `/home/croyse/calyx/data/fsv-issue373-temporal-policy-reopen-20260609-a54dcc1`
- Additional proof: `BoostConfig.post_retrieval_alpha` is serialized,
  defaulted for older manifests, capped at 0.10, and a custom temporal policy
  survives cold open plus second flush instead of being replaced by defaults.
- T02 #374 commit: `d872c7c`
- aiwonder FSV root:
  `/home/croyse/calyx/data/fsv-issue374-time-window-20260609-d872c7c`
- Source of truth: `temporal-window-input.json`,
  `temporal-window-readback.json`, and `BLAKE3SUMS.txt` under the FSV root.
  Readback keeps only hit IDs 01 and 03 for window `[992800, 1000000)`, proving
  the out-of-window hit 02 is absent and retained order is unchanged. Edge
  proofs cover empty input, all-window retention of missing timestamps, and
  `CALYX_TEMPORAL_INVALID_WINDOW` for zero, reversed, and overflow windows.
- T03 #375 commit: `a54dcc1`
- aiwonder FSV root:
  `/home/croyse/calyx/data/fsv-issue375-temporal-boost-20260609-a54dcc1`
- Source of truth: `temporal-boost-input.json`,
  `temporal-boost-readback.json`, and `BLAKE3SUMS.txt` under the FSV root.
  Readback shows the high-content old hit remains rank 1 after a content-
  relative temporal boost, `TemporalScores` are attached for explain output,
  and the zero-content recent hit remains score 0.0. Edge proofs cover empty
  input, single-hit E4 = 1.0, missing timestamps, and
  `CALYX_TEMPORAL_AP60_VIOLATION` for `never_dominant=false`.
- T04 #376 commit: `78f9b67`
- aiwonder FSV root:
  `/home/croyse/calyx/data/fsv-issue376-causal-gate-20260609-78f9b67`
- Source of truth: `causal-gate-input.json`,
  `causal-gate-readback.json`, and `BLAKE3SUMS.txt` under the FSV root.
  Readback shows the final `temporal_search_pipeline` ranking after
  window-filter -> temporal boost -> causal gate. High confidence hit 01 reads
  back score `1.0642499923706055`, neutral hit 02 reads back
  `0.8506667017936707`, and low hit 03 reads back `0.6257416605949402`.
  Each hit carries `causal_confidence` and `causal_gate` explain evidence.
  Edge proofs cover empty input, `Absent` confidence treated as multiplier 1.0,
  and `CALYX_TEMPORAL_INVALID_BOOST_CONFIG` for negative and over-10 causal
  multipliers. `BLAKE3SUMS.txt` digest:
  `aca9fc8102bd40b6c9f7c8f113fd39b72da633b00edef4486dd13a2d4527d3e7`.
- T05 #377 commit: `b428b10`
- aiwonder FSV root:
  `/home/croyse/calyx/data/fsv-issue377-temporal-search-20260610-b428b10`
- Source of truth: `temporal-search-input.json`,
  `temporal-search-readback.json`, `temporal-search-cli-readback.json`, and
  `BLAKE3SUMS.txt` under the FSV root. CLI readback was produced by
  `calyx readback temporal_search --explain --clock-fixed 1000000 --tz-offset 0`.
  Readback shows `pre_boost_ranking` IDs 01, 02, 03; final hits 02, 01;
  `temporal_weight_used = 0.0`; `primary_slots_used = [8]`; and
  `temporal_slots_excluded = [20]`. The recent content-miss ID 03 is absent
  from the final `k=2` results while still visible in the pre-boost recall set.
  Edge proofs cover empty vault output, one-hour window exclusion of the old
  hit, zero-content score remaining `0.0`, UTC-5 periodic score `0.5` versus
  UTC score `0.0`, and `CALYX_TEMPORAL_AP60_VIOLATION` for non-zero primary
  temporal weight.
- T06 #378 commit: `2205edb`
- aiwonder FSV root:
  `/home/croyse/calyx/data/fsv-issue378-temporal-never-dominant-20260610-2205edb`
- Source of truth: `temporal-never-dominant-input.json`,
  `temporal-never-dominant-readback.json`, and `BLAKE3SUMS.txt` under the FSV
  root. Separate after-read ran `b3sum -c BLAKE3SUMS.txt` and opened both JSON
  files. BLAKE3 input digest:
  `db7a5bcea78d9037ace122fd1d326895277814bcf6485544f0bc05518e098ef1`;
  readback digest:
  `5e466e6d072bd32bc6b1fabe8d49a9d5a7422982583526a891ad8e3a02922ff0`.
- Readback proves the content-miss ID 03 remains score `0.0` and absent from
  `positive_surface_ids`; close content matches reorder from `[0a, 0b]` to
  `[0b, 0a]` with old score `0.6831000447273254` and fresh score
  `0.690329909324646`; raw retrieval temporal weight is `0.0`; invalid weight
  `0.25` returns `CALYX_TEMPORAL_AP60_VIOLATION`; E2 reads `0.5` by query time
  rather than ingest-relative `0.75`; E3 reads `0.5` under UTC-5 and `0.0`
  under UTC; the all-zero edge has `after_positive_surface_count = 0`.
- Post-sweep hardening #615 commit: `b9a105c`
- aiwonder FSV root:
  `/home/croyse/calyx/data/fsv-issue615-ap60-final-surface-20260610-b9a105c`
- Source of truth: `temporal-search/temporal-search-input.json`,
  `temporal-search/temporal-search-readback.json`,
  `temporal-never-dominant/temporal-never-dominant-input.json`,
  `temporal-never-dominant/temporal-never-dominant-readback.json`, and both
  `BLAKE3SUMS.txt` files under the #615 root. Separate after-read ran
  `b3sum -c`, opened all four JSON files, and parsed the invariant fields.
- Hashes: temporal-search input
  `889e95f471f36481c83fb69a2d362de4b25f1b809e7c68fe440768c109d8f3c4`;
  temporal-search readback
  `4ac1f39146d2d64113285d5e691a08fe886a9fb6836c1e2a3fc7c6af96fb08b1`;
  temporal-never-dominant input
  `db7a5bcea78d9037ace122fd1d326895277814bcf6485544f0bc05518e098ef1`;
  temporal-never-dominant readback
  `b6c88b8d26c5ddb2f3292c37034f5ee740ea7cdfd92f1cfaaca873b8f7a8e41b`.
- #615 readback proves the final `temporal_search` surface excludes
  non-positive hits: `actual_pre_boost = [1, 2, 3]`,
  `actual_final = [2, 1]`, `content_miss_absent_from_final = true`,
  `zero_content_edge.boost_after_score = 0.0`, and
  `zero_content_edge.final_surface_contains_zero_content = false`. The all-zero
  edge now reads `final_after_hits = []` and
  `after_positive_surface_count = 0`.

## FSV exit gate (the phase is DONE only when this is byte-proven on aiwonder)

A recent/periodic item that does NOT match a content lens must **not** surface in
results — temporal never dominant. PH40 readbacks record the ranked result list
before and after `apply_temporal_boost`/`temporal_search`, confirming the boost
only reorders content-matching hits, keeps content misses at `score = 0.0`, and
filters non-positive hits from the final temporal-search result. `temporal
weight = 0.0` is visible in raw retrieval/explain output. T03 proves the pure
boost helper bytes; T05/T06/#615 prove the full pipeline and exit-gate
invariants on aiwonder with an injected fixed clock and synthetic result sets
where the content-miss is the most recent item.

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
- **T05 integration ordering:** fusion still creates hits before provenance
  timestamps are attached. The full temporal pipeline must overfetch, attach
  provenance/event time, apply the window/boost/gate stages, then final-truncate
  and renumber.
