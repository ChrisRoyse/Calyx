# PH40 · T05 — AP-60 invariant enforcement + `temporal_search` integration

| Field | Value |
|---|---|
| **Phase** | PH40 — Temporal Fusion + AP-60 Post-Retrieval Boost |
| **Stage** | S9 — Temporal & Dedup |
| **Crate** | `calyx-sextant` |
| **Files** | `crates/calyx-sextant/src/temporal/search.rs` (≤500), `crates/calyx-cli/src/temporal_readback.rs` (≤500), temporal-search tests (≤500 each) |
| **Depends on** | T04 (this phase) · PH24 (search entry point) |
| **Axioms** | A27 |
| **PRD** | `dbprdplans/25 §3`, `dbprdplans/25 §8` |

## Goal

Expose the `temporal_search` public API that wraps PH24's primary retrieval with
the temporal post-retrieval pipeline. The function signature enforces the AP-60
invariant at the boundary: temporal weight is 0.0 in the primary ANN retrieval
call; the boost pipeline (T03+T04) is applied only after retrieval returns. E2
age is computed relative to query-time. E3 scoring is timezone-aware. The explain
output surfaces the before/after ranked lists so FSV can be performed.

## Build (checklist of concrete, code-level steps)

- [x] Implement `temporal_search(vault, query, window: Option<TimeWindow>, policy: &TemporalPolicy, clock: &dyn Clock, tz_offset_secs: i32) -> Result<TemporalSearchResult, CalyxError>`:
  - call PH24 `search(vault, query, temporal_weight=0.0)` → raw ranked `Vec<Hit>` (temporal excluded from primary ANN)
  - record `pre_boost_ranking: Vec<CxId>` for explain
  - if `window.is_some()` → `filter_hits_by_window`
  - `apply_temporal_boost(filtered, policy, clock.now_secs(), tz_offset_secs)`
  - `apply_causal_gate(boosted, &policy.boost)`
  - filter non-positive hits from the final search surface
  - return `TemporalSearchResult { hits, pre_boost_ranking, policy_snapshot }`
- [x] Define `TemporalSearchResult { hits: Vec<Hit>, pre_boost_ranking: Vec<CxId>, policy_snapshot: TemporalPolicy }` with `serde` + `Debug`
- [x] Enforce: inside `temporal_search`, assert that the primary retrieval call passes `temporal_weight = 0.0`; if the search backend returns a `temporal_weight_used` field, assert it is 0.0 → `CALYX_TEMPORAL_AP60_VIOLATION` otherwise
- [x] E2 age must use `clock.now_secs()` as query-time, not any field from the vault metadata
- [x] E3 must receive `tz_offset_secs` from the caller; no silent UTC assumption in integration code
- [x] Expose `temporal_search` from `calyx-sextant` lib root

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: `temporal_search` on a 3-hit vault with `FixedClock { secs: 1_000_000 }`, no window -> pre-boost ranking records all 3 primary hits; final surface contains only the 2 positive content hits after boost/gate
- [x] unit: `temporal_search` with a window that excludes 1 of 3 hits -> result contains only the positive in-window hit after the zero-content final-surface filter
- [x] unit: hit with `content_score = 0.0` in primary results -> boost-stage proof remains `0.0` and final `temporal_search` surface excludes it
- [x] proptest: `temporal_search` result hit IDs are a subset of primary retrieval IDs (no hallucinated hits)
- [x] edge: vault with 0 constellations → empty result, no panic
- [x] edge: `tz_offset_secs = -18000` (UTC-5) → E3 hour scoring uses local hour, not UTC hour; verify with a hit at UTC 19:00 = local 14:00 matching `target_hour=14`
- [x] fail-closed: if primary retrieval returns `temporal_weight_used > 0.0` → `CALYX_TEMPORAL_AP60_VIOLATION` propagated
- [x] fail-loud: mixed empty/non-empty primary slots propagate `CALYX_SEXTANT_INDEX_EMPTY` instead of pretending the vault is empty

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `TemporalSearchResult` JSON written to stdout by `calyx readback temporal_search --explain`
- **Readback:** `calyx readback temporal_search --explain --clock-fixed 1_000_000 --tz-offset 0` on a two-constellation vault (one recent content-match, one old content-match, one recent content-miss); print `pre_boost_ranking` and final `hits`
- **Prove:** (a) `pre_boost_ranking` shows content-score order; (b) temporal boost may reorder among content-matches; (c) content-miss (score=0.0) is absent from results regardless of recency; (d) `policy_snapshot.never_dominant = true` visible in output
- **Completed on aiwonder:** commit `b428b10`; evidence root
  `/home/croyse/calyx/data/fsv-issue377-temporal-search-20260610-b428b10`.
  Files read back: `temporal-search-input.json`,
  `temporal-search-readback.json`, `temporal-search-cli-readback.json`,
  and `BLAKE3SUMS.txt`.
- **Post-sweep hardening:** #615 commit `b9a105c`; evidence root
  `/home/croyse/calyx/data/fsv-issue615-ap60-final-surface-20260610-b9a105c`.
  Separate after-read verified `temporal-search/BLAKE3SUMS.txt`, opened
  `temporal-search-input.json` and `temporal-search-readback.json`, and read
  back `actual_pre_boost = [1, 2, 3]`, `actual_final = [2, 1]`,
  `content_miss_absent_from_final = true`,
  `zero_content_edge.boost_after_score = 0.0`, and
  `zero_content_edge.final_surface_contains_zero_content = false`.

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] FSV evidence (readback output / screenshot) attached to GitHub issue #377
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
