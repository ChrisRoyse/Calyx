# PH42 · T05 — Sextant: frequency/recency recurrence boost (AP-60)

| Field | Value |
|---|---|
| **Phase** | PH42 — Grounded Recurrence Wiring Across Engines |
| **Stage** | S9 — Temporal & Dedup |
| **Crate** | `calyx-sextant` |
| **Files** | `crates/calyx-sextant/src/temporal/recurrence_boost.rs` (≤500) |
| **Depends on** | T01 (this phase) · PH40 (temporal boost pipeline) · PH41 (frequency + series) |
| **Axioms** | A29, A27 |
| **PRD** | `dbprdplans/25 §3`, `dbprdplans/25 §4c` |

## Goal

Wire frequency and recency-of-last-occurrence into the PH40 Sextant AP-60
post-retrieval boost. A constellation that recurs frequently AND was observed
recently should receive a mild additional boost — but still bounded by AP-60:
temporal and recurrence signals combined must never be dominant over content
score. The recurrence boost is an additive term on top of the E2/E3/E4 boost,
capped so that zero-content-score hits remain at zero.

## Build (checklist of concrete, code-level steps)

- [ ] Define `RecurrenceBoostConfig { frequency_weight: f32, recency_weight: f32, max_recurrence_boost: f32 }` — defaults: `frequency_weight=0.05`, `recency_weight=0.05`, `max_recurrence_boost=0.10` (so total recurrence contribution ≤ 0.10 of content score)
- [ ] Implement `recurrence_boost_score(cx_id: CxId, vault: &Vault, query_time_secs: i64, config: &RecurrenceBoostConfig) -> Result<f32, CalyxError>`:
  - read `frequency` from base CF (O(1))
  - read `last_occurrence_t` = max `t_k` from series (or `None` if no occurrences)
  - `freq_component = frequency_kernel_bonus(frequency) * config.frequency_weight` (reuse T03 formula)
  - `recency_component = score_e2_recency(last_occurrence_t, query_time_secs, decay=Exponential{half_life=3600}) * config.recency_weight`
  - `total = (freq_component + recency_component).min(config.max_recurrence_boost)`
  - return `total`
- [ ] Integrate into `apply_temporal_boost` (PH40 T03): after computing `fuse_temporal`, add `recurrence_boost_score` to the fused score; the AP-60 zero-content-score guard still applies last (zero content → zero total boost)
- [ ] `RecurrenceBoostConfig` is part of `TemporalPolicy`; add it as an optional field (`recurrence_boost: Option<RecurrenceBoostConfig>`) — `None` = recurrence boost disabled (default: `Some(default_config)`)
- [ ] Frequency-based boost is read-only from the base CF; no write path in Sextant

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: `frequency=0` → `freq_component = 0.0`; `last_occurrence = None` → `recency_component = 0.0`; `total = 0.0`
- [ ] unit: `frequency=10`, `last_occurrence = query_time - 1800` (30m ago), half_life=3600: `recency ≈ exp(-0.5*0.693) ≈ 0.707`; `recency_component = 0.707 * 0.05 ≈ 0.035`; `freq_component ≈ freq_bonus(10) * 0.05`; `total = min(sum, 0.10)`
- [ ] unit: zero-content-score hit with recurrence boost = 0.08 → final score remains 0.0 (AP-60 guard applied after recurrence boost)
- [ ] unit: `max_recurrence_boost = 0.10` cap: very high frequency + very recent → `total = 0.10` (capped)
- [ ] unit: `TemporalPolicy { recurrence_boost: None }` → `recurrence_boost_score` not called; output identical to PH40-only pipeline
- [ ] proptest: `recurrence_boost_score ∈ [0.0, max_recurrence_boost]` for all valid inputs
- [ ] edge: `frequency = u64::MAX` → `freq_component` capped at `frequency_weight` (bonus=1.0); no overflow
- [ ] fail-closed: base CF read error → `CALYX_SEXTANT_RECURRENCE_READ_ERROR`

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `TemporalSearchResult` with `recurrence_boost` field in explain output
- **Readback:** `calyx readback temporal_search --explain --clock-fixed 1_000_000` on a vault with CxId-A (frequency=50, content_score=0.7) and CxId-B (frequency=1, content_score=0.7); print post-boost scores
- **Prove:** A's final score > B's final score (frequency boost applied); difference = recurrence contribution ≈ expected arithmetic; explain output shows `recurrence_boost: <value>` field

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH42 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
