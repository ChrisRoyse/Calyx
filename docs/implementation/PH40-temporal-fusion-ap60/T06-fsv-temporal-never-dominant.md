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
the temporal-search artifacts on aiwonder.

## Build (checklist of concrete, code-level steps)

- [ ] Create `tests.rs` module in `crates/calyx-sextant/src/temporal/`; gate with `#[cfg(test)]`
- [ ] `fsv_temporal_never_dominant`: construct a synthetic `Vec<Hit>` with three hits: (A) content_score=0.8, age=1h; (B) content_score=0.6, age=30m; (C) content_score=0.0, age=5m (extremely recent content-miss). Run `temporal_search_pipeline` with `FixedClock`. Assert: C is absent OR its score remains 0.0 (never elevated by temporal boost). Assert A and B remain in result set.
- [ ] `fsv_boost_reorders_content_matches`: construct two hits — (A) content_score=0.7, age=24h (old); (B) content_score=0.65, age=10m (very fresh). Pre-boost order: A then B (by content). Run pipeline with Exponential decay half_life=3600, fusion_weights=default. Assert post-boost score_B > score_A (boost elevated B) — demonstrates reordering among content-matching hits. Assert neither score exceeds 1.0 + boost_alpha (no runaway scores).
- [ ] `fsv_ap60_weight_zero_in_retrieval`: mock the PH24 search call; assert `temporal_weight` argument passed to it is exactly 0.0f32 — capture via a test-double that records the argument
- [ ] `fsv_e2_uses_query_time_not_ingest_time`: construct a hit with `event_time = 1_000_000`, `ingest_time = 1_100_000` (ingested 100_000s after event). Set `clock.now_secs() = 1_200_000`. Assert E2 age = 200_000 (query_time − event_time), NOT 100_000 (query_time − ingest_time).
- [ ] `fsv_e3_timezone_aware`: hit at UTC epoch corresponding to 19:00 UTC (= 14:00 UTC-5). Run E3 with `tz_offset_secs = -18000`, `target_hour = 14`. Assert score = 0.5 (hour match). Re-run with `tz_offset_secs = 0`, `target_hour = 14`. Assert score = 0.0 (no UTC match). Both same hit, different tz context.
- [ ] Each test is `#[test]`, seeded, deterministic; `FixedClock` used throughout; no `SystemTime::now()`

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] `fsv_temporal_never_dominant` passes with assertion on C's score
- [ ] `fsv_boost_reorders_content_matches` passes with assertion on post-boost ordering
- [ ] `fsv_ap60_weight_zero_in_retrieval` passes by capturing the search argument
- [ ] `fsv_e2_uses_query_time_not_ingest_time` passes with exact age calculation
- [ ] `fsv_e3_timezone_aware` passes with both tz variants
- [ ] proptest: for any `Vec<Hit>` with at least one zero-content-score hit, after pipeline that hit's score remains 0.0 (AP-60 property holds universally)
- [ ] edge: all hits have zero content score → empty result (no temporal surfacing)
- [ ] fail-closed: injecting a mock that sets `temporal_weight > 0.0` in retrieval → `CALYX_TEMPORAL_AP60_VIOLATION` caught in FSV test

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** temporal-search before/after ranked-list readback artifacts on aiwonder
- **Readback:** run the deterministic temporal-search FSV trigger, then
  separately `cat`/`b3sum -c` the input, pre-boost, post-boost, and edge-case
  JSON artifacts under the #378 FSV root; paste the after-read bytes to GitHub
  issue #378
- **Prove:** content-miss score after boost is physically present as `0.0`,
  close content matches can reorder post-boost, raw retrieval still records
  temporal weight `0.0`, E2 age uses query time, and E3 changes when the
  timezone offset changes

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to GitHub issue #378
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
