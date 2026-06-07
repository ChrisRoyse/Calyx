# PH40 · T05 — AP-60 invariant enforcement + `temporal_search` integration

| Field | Value |
|---|---|
| **Phase** | PH40 — Temporal Fusion + AP-60 Post-Retrieval Boost |
| **Stage** | S9 — Temporal & Dedup |
| **Crate** | `calyx-sextant` |
| **Files** | `crates/calyx-sextant/src/temporal/mod.rs` (≤500) |
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

- [ ] Implement `temporal_search(vault, query, window: Option<TimeWindow>, policy: &TemporalPolicy, clock: &dyn Clock, tz_offset_secs: i32) -> Result<TemporalSearchResult, CalyxError>`:
  - call PH24 `search(vault, query, temporal_weight=0.0)` → raw ranked `Vec<Hit>` (temporal excluded from primary ANN)
  - record `pre_boost_ranking: Vec<CxId>` for explain
  - if `window.is_some()` → `filter_hits_by_window`
  - `apply_temporal_boost(filtered, policy, clock.now_secs(), tz_offset_secs)`
  - `apply_causal_gate(boosted, &policy.boost)`
  - return `TemporalSearchResult { hits, pre_boost_ranking, policy_snapshot }`
- [ ] Define `TemporalSearchResult { hits: Vec<Hit>, pre_boost_ranking: Vec<CxId>, policy_snapshot: TemporalPolicy }` with `serde` + `Debug`
- [ ] Enforce: inside `temporal_search`, assert that the primary retrieval call passes `temporal_weight = 0.0`; if the search backend returns a `temporal_weight_used` field, assert it is 0.0 → `CALYX_TEMPORAL_AP60_VIOLATION` otherwise
- [ ] E2 age must use `clock.now_secs()` as query-time, not any field from the vault metadata
- [ ] E3 must receive `tz_offset_secs` from the caller; no silent UTC assumption in integration code
- [ ] Expose `temporal_search` from `calyx-sextant` lib root

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: `temporal_search` on a 3-hit vault with `FixedClock { secs: 1_000_000 }`, no window → pre-boost ranking recorded, post-boost ranking may differ; assert `pre_boost_ranking.len() == hits.len()`
- [ ] unit: `temporal_search` with a window that excludes 1 of 3 hits → result contains 2 hits only
- [ ] unit: hit with `content_score = 0.0` in primary results → not present in boosted output with elevated score (AP-60)
- [ ] proptest: `temporal_search` result hit IDs are a subset of primary retrieval IDs (no hallucinated hits)
- [ ] edge: vault with 0 constellations → empty result, no panic
- [ ] edge: `tz_offset_secs = -18000` (UTC-5) → E3 hour scoring uses local hour, not UTC hour; verify with a hit at UTC 19:00 = local 14:00 matching `target_hour=14`
- [ ] fail-closed: if primary retrieval returns `temporal_weight_used > 0.0` → `CALYX_TEMPORAL_AP60_VIOLATION` propagated

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `TemporalSearchResult` JSON written to stdout by `calyx readback temporal_search --explain`
- **Readback:** `calyx readback temporal_search --explain --clock-fixed 1_000_000 --tz-offset 0` on a two-constellation vault (one recent content-match, one old content-match, one recent content-miss); print `pre_boost_ranking` and final `hits`
- **Prove:** (a) `pre_boost_ranking` shows content-score order; (b) temporal boost may reorder among content-matches; (c) content-miss (score=0.0) is absent from results regardless of recency; (d) `policy_snapshot.never_dominant = true` visible in output

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH40 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
