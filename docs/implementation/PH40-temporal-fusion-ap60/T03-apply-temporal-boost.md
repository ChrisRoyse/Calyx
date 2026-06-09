# PH40 · T03 — `apply_temporal_boost` post-retrieval reranker

| Field | Value |
|---|---|
| **Phase** | PH40 — Temporal Fusion + AP-60 Post-Retrieval Boost |
| **Stage** | S9 — Temporal & Dedup |
| **Crate** | `calyx-sextant` |
| **Files** | `crates/calyx-sextant/src/temporal/boost.rs` (≤500) |
| **Depends on** | T02 (this phase) · PH24 (Hit type + ranked results) |
| **Axioms** | A27 |
| **PRD** | `dbprdplans/25 §3`, `dbprdplans/25 §2` |

## Goal

Implement `apply_temporal_boost` — the function that takes a ranked `Vec<Hit>`
returned by PH24 fusion and applies E2/E3/E4 scores as a weighted post-retrieval
boost. The boost adjusts existing content-similarity scores but never promotes a
hit to rank #1 if it had no content match (AP-60). E2 age is computed relative
to `query_time` (not ingest time). E3 requires a timezone offset for correct
hour/dow extraction. E4 uses each hit's sequence position within the result set.
Fusion weighting: 50% recency (E2) / 35% sequence (E4) / 15% periodic (E3).

## Build (checklist of concrete, code-level steps)

- [ ] Define `TemporalScores { e2_recency: f32, e3_periodic: f32, e4_sequence: f32 }` — per-hit temporal subscores (attached to `Hit` for explain output)
- [ ] Implement `score_e2_recency(event_time_secs: i64, query_time_secs: i64, decay: &DecayFunction) -> f32`:
  - `age_secs = query_time_secs − event_time_secs` (must be ≥0; negative age → 1.0 clamp)
  - `Linear`: `(1.0 − age_secs as f32 / max_age_secs as f32).max(0.0)`
  - `Exponential`: `(-age_secs as f32 * 0.693 / half_life_secs as f32).exp()`
  - `Step`: age <3600 → 0.8; <86400 → 0.5; else → 0.1
- [ ] Implement `score_e3_periodic(event_time_secs: i64, opts: &PeriodicOptions, tz_offset_secs: i32) -> f32`: extract local hour = `((event_time_secs + tz_offset_secs as i64) % 86400 / 3600) as u8`; dow = day-of-week mod 7; score += 0.5 per matching target (max 1.0)
- [ ] Implement `score_e4_sequence(rank: usize, total: usize) -> f32`: positional score = `1.0 − rank as f32 / total as f32` (rank 0-indexed; single-result → 1.0)
- [ ] Implement `fuse_temporal(scores: &TemporalScores, weights: &FusionWeights) -> f32`: `weights.recency * e2 + weights.sequence * e4 + weights.periodic * e3`
- [ ] Implement `apply_temporal_boost(hits: Vec<Hit>, policy: &TemporalPolicy, query_time_secs: i64, tz_offset_secs: i32) -> Vec<Hit>`:
  - assert `policy.never_dominant == true` → `CALYX_TEMPORAL_AP60_VIOLATION` if false
  - for each hit: compute `TemporalScores`, fuse → `t_score`; `new_score = content_score + t_score * boost_alpha` where `boost_alpha` is a small configurable scalar (default 0.1) ensuring temporal is never dominant
  - re-sort descending by `new_score`; attach `TemporalScores` to hit for explain
  - hits with zero content score are NOT boosted (AP-60: temporal cannot surface a content miss)
- [ ] All scoring functions must be `#[inline]` and pure (no side effects, no I/O)

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: `score_e2_recency(event=1000, query=4600, decay=Linear{max_age=3600})` → age=3600, score=0.0
- [ ] unit: `score_e2_recency(event=1000, query=1900, decay=Step)` → age=900 (<3600) → 0.8
- [ ] unit: `score_e3_periodic(t=Tuesday_14h_utc, opts={target_day_of_week:Some(1), target_hour:Some(14)}, tz=0)` → 1.0
- [ ] unit: `score_e4_sequence(rank=0, total=5)` → 1.0; `rank=4, total=5` → 0.2
- [ ] unit: `apply_temporal_boost` on two hits where hit-A has higher content score but older event, hit-B has lower content score but is recent — A must remain rank #1 (AP-60: boost cannot flip a content-dominant ranking by more than the alpha cap)
- [ ] unit: hit with `content_score = 0.0` → boost is NOT applied (remains 0.0 after boost pass)
- [ ] proptest: `fuse_temporal` output is in `[0.0, 1.0]` for all valid input scores and default weights
- [ ] edge: empty hit list → returns empty vec without panic
- [ ] edge: single hit → `score_e4_sequence(0, 1)` → 1.0
- [ ] fail-closed: `policy.never_dominant = false` → `CALYX_TEMPORAL_AP60_VIOLATION` (cannot reach boost logic)

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** ranked hit list before and after `apply_temporal_boost`, read from a deterministic synthetic vault
- **Readback:** `calyx readback temporal_search --explain --clock-fixed 1_000_000` on a vault with two constellations — one high-content-score/old, one low-content-score/recent; print scores before and after boost
- **Prove:** (a) pre-boost ranking: high-content item rank #1; (b) post-boost: high-content item still rank #1 (AP-60 held); (c) `TemporalScores` fields visible in explain output; (d) zero-content-score hit shows temporal = 0.0

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to GitHub issue #375
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
