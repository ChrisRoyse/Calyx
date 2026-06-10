# PH40 · T06 — FSV: temporal-never-dominant + boost-reorder proof

| Field | Value |
|---|---|
| **Phase** | PH40 — Temporal Fusion + AP-60 Post-Retrieval Boost |
| **Stage** | S9 — Temporal & Dedup |
| **Crate** | `calyx-sextant` |
| **Files** | `crates/calyx-sextant/src/temporal/tests.rs` (≤500) |
| **Depends on** | T05 (this phase) |
| **Axioms** | A27 |
| **PRD** | `dbprdplans/25 §3`, `dbprdplans/25 §2` |

## Goal

Write the deterministic FSV test suite that proves the two core PH40 invariants
byte-by-byte on aiwonder: (1) temporal is never dominant — a content-miss item
cannot surface regardless of recency; (2) the boost correctly reorders
content-matching hits post-retrieval (before/after ranking delta observable).
These tests support the phase gate; the formal FSV verdict is the after-read of
the temporal-never-dominant artifacts on aiwonder.

## Build (checklist of concrete, code-level steps)

- [x] Create `tests.rs` module in `crates/calyx-sextant/src/temporal/`; gate with `#[cfg(test)]`
- [x] `fsv_temporal_never_dominant`: construct a synthetic `Vec<Hit>` with three hits: (A) content_score=0.8, age=1h; (B) content_score=0.6, age=30m; (C) content_score=0.0, age=5m (extremely recent content-miss). Run `temporal_search_pipeline` with `FixedClock`. Assert: C is absent OR its score remains 0.0 (never elevated by temporal boost). Assert A and B remain in result set.
- [x] `fsv_boost_reorders_content_matches`: construct two close content matches — (A) content_score=0.66, age=24h (old); (B) content_score=0.65, age=10m (very fresh). Pre-boost order: A then B (by content). Run pipeline with Exponential decay half_life=3600, fusion_weights=default. Assert post-boost score_B > score_A (boost elevated B) — demonstrates reordering among content-matching hits. Assert neither score exceeds 1.0 + boost_alpha (no runaway scores). The close-score pair is required because AP-60's boost is intentionally bounded and should not overpower a wider content gap.
- [x] `fsv_ap60_weight_zero_in_retrieval`: assert `temporal_weight` used at the PH24 retrieval boundary is exactly `0.0f32`, and assert non-zero injection returns `CALYX_TEMPORAL_AP60_VIOLATION`
- [x] `fsv_e2_uses_query_time_not_ingest_time`: construct a hit with `event_time = 1_000_000`, `ingest_time = 1_100_000` (ingested 100_000s after event). Set `clock.now_secs() = 1_200_000`. Assert E2 age = 200_000 (query_time - event_time), NOT 100_000 (query_time - ingest_time).
- [x] `fsv_e3_timezone_aware`: hit at UTC epoch corresponding to 19:00 UTC (= 14:00 UTC-5). Run E3 with `tz_offset_secs = -18000`, `target_hour = 14`. Assert score = 0.5 (hour match). Re-run with `tz_offset_secs = 0`, `target_hour = 14`. Assert score = 0.0 (no UTC match). Both same hit, different tz context.
- [x] Each test is `#[test]`, seeded, deterministic; `FixedClock` used throughout; no `SystemTime::now()`

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] `fsv_temporal_never_dominant` passes with assertion on C's score
- [x] `fsv_boost_reorders_content_matches` passes with assertion on post-boost ordering
- [x] `fsv_ap60_weight_zero_in_retrieval` passes by validating the search argument
- [x] `fsv_e2_uses_query_time_not_ingest_time` passes with exact age calculation
- [x] `fsv_e3_timezone_aware` passes with both tz variants
- [x] proptest: for any `Vec<Hit>` with at least one zero-content-score hit, after pipeline that hit's score remains 0.0 (AP-60 property holds universally)
- [x] edge: all hits have zero content score -> no positive temporal surfacing
- [x] fail-closed: injecting `temporal_weight > 0.0` in retrieval -> `CALYX_TEMPORAL_AP60_VIOLATION` caught in FSV test

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** temporal-search before/after ranked-list readback artifacts on aiwonder
- **Commit:** `2205edb` (`Add temporal never dominant FSV proofs`)
- **FSV root:**
  `/home/croyse/calyx/data/fsv-issue378-temporal-never-dominant-20260610-2205edb`
- **Readback:** the deterministic FSV trigger wrote
  `temporal-never-dominant-input.json`,
  `temporal-never-dominant-readback.json`, and `BLAKE3SUMS.txt`. A separate
  aiwonder after-read listed the files, ran `b3sum -c BLAKE3SUMS.txt`, and
  opened both JSON files with `cat`.
- **Hashes:** input
  `db7a5bcea78d9037ace122fd1d326895277814bcf6485544f0bc05518e098ef1`;
  readback
  `5e466e6d072bd32bc6b1fabe8d49a9d5a7422982583526a891ad8e3a02922ff0`.
- **Prove:** content-miss score after boost is physically present as `0.0`,
  close content matches can reorder post-boost, raw retrieval still records
  temporal weight `0.0`, E2 age uses query time, and E3 changes when the
  timezone offset changes
- **Observed bytes:** `positive_surface_ids` contains only IDs 01 and 02;
  the content-miss ID 03 remains present only with `score = 0.0`; reorder
  changes from `[0a, 0b]` to `[0b, 0a]` with old score
  `0.6831000447273254` and fresh score `0.690329909324646`; invalid temporal
  retrieval weight `0.25` returns `CALYX_TEMPORAL_AP60_VIOLATION`; E2 reads
  `0.5` versus ingest-relative wrong value `0.75`; E3 reads `0.5` under UTC-5
  and `0.0` under UTC; the all-zero edge has `after_positive_surface_count = 0`.
- **Post-sweep hardening:** #615 commit `b9a105c`; evidence root
  `/home/croyse/calyx/data/fsv-issue615-ap60-final-surface-20260610-b9a105c`.
  Separate after-read verified `temporal-never-dominant/BLAKE3SUMS.txt`,
  opened `temporal-never-dominant-input.json` and
  `temporal-never-dominant-readback.json`, and read back
  `all_zero_edge.boost_after_hits` with both scores `0.0`,
  `all_zero_edge.final_after_hits = []`, and
  `all_zero_edge.after_positive_surface_count = 0`. Readback hash:
  `b6c88b8d26c5ddb2f3292c37034f5ee740ea7cdfd92f1cfaaca873b8f7a8e41b`.

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) <= 500 lines (line-count gate)
- [x] FSV evidence (readback output) captured for GitHub issue #378
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
