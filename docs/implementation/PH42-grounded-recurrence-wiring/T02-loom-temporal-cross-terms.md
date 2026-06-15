# PH42 · T02 — Loom: temporal cross-terms + co-occurrence lead-lag

| Field | Value |
|---|---|
| **Phase** | PH42 — Grounded Recurrence Wiring Across Engines |
| **Stage** | S9 — Temporal & Dedup |
| **Crate** | `calyx-loom` |
| **Files** | `crates/calyx-loom/src/recurrence/cross_terms.rs` (≤500) |
| **Depends on** | T01 (this phase) · PH27 (agreement graph + cross-terms) · PH41 (recurrence series) |
| **Axioms** | A29, A8 |
| **PRD** | `dbprdplans/25 §4c`, `dbprdplans/06 §3`, `dbprdplans/08 §2` |

## Goal

Extend Loom's agreement graph (PH27) with temporal cross-terms: associations
between two constellations' recurrence patterns. The key signal is
**co-occurrence lead-lag**: if CxId-A tends to recur shortly before CxId-B
recurs, that is a grounded directional association — the raw material for
causality (Stage 11). Compute `lead_lag_secs(A, B)` as the median time delta
`t_k(B) - t_k(A)` over co-occurring occurrence pairs within a proximity window.
A positive `lead_lag` means A leads B (A precedes B); this is raw association
data, NOT yet causality claims.

## Build (checklist of concrete, code-level steps)

- [ ] Define `LeadLagResult { cx_a: CxId, cx_b: CxId, lead_lag_secs: f64, n_pairs: usize, proximity_window_secs: u64 }` — `lead_lag_secs > 0` means A leads B; `< 0` means B leads A
- [ ] Implement `co_occurrence_pairs(series_a: &RecurrenceSeries, series_b: &RecurrenceSeries, window_secs: u64) -> Vec<(EpochSecs, EpochSecs)>`:
  - for each occurrence `t_a` in A: find all occurrences `t_b` in B such that `|t_b - t_a| < window_secs`
  - return list of `(t_a, t_b)` pairs
- [ ] Implement `lead_lag_secs(series_a: &RecurrenceSeries, series_b: &RecurrenceSeries, window_secs: u64) -> Option<LeadLagResult>`:
  - call `co_occurrence_pairs`; if `n_pairs < 3` → `None` (insufficient)
  - `deltas: Vec<f64>` = `t_b - t_a` for each pair
  - `lead_lag = median(deltas)` (stable, seeded sort for determinism)
  - return `Some(LeadLagResult { ..., lead_lag_secs: lead_lag, n_pairs })`
- [ ] Implement `temporal_cross_term(cx_a: CxId, cx_b: CxId, vault: &Vault, window_secs: u64) -> Result<Option<LeadLagResult>, CalyxError>`:
  - read both series from T05 `SeriesStore`; compute `lead_lag_secs`; return result
- [ ] Store `LeadLagResult` in a new `temporal_xterm` CF under key `(cx_a, cx_b)` (lexicographic, so `(A,B)` and `(B,A)` are stored once each with different sign); write via WAL
- [ ] Extend Loom's existing cross-term API (PH27) with `temporal_cross_term` as a new cross-term type; do NOT replace existing MI-based cross-terms
- [ ] Raw material only: do NOT attach a causality label; that is Stage 11's job

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: A recurs at [100, 200, 300], B recurs at [110, 210, 310], window=30 → 3 pairs, all deltas = 10s → `lead_lag_secs = 10.0`, A leads B
- [ ] unit: A recurs at [100, 200, 300], B recurs at [90, 190, 290], window=30 → deltas = -10s → `lead_lag_secs = -10.0`, B leads A
- [ ] unit: A has 5 occurrences, B has 2 occurrences, no pairs within window=5s → `None`
- [ ] unit: stored `LeadLagResult` read back from `temporal_xterm` CF byte-exact (round-trip via `xxd`)
- [ ] unit: `co_occurrence_pairs` is O(n·m) but gated by `n_pairs < 3` short-circuit (test that insufficient case returns None quickly)
- [ ] proptest: `lead_lag_secs(A, B) = -lead_lag_secs(B, A)` (sign flip for reversed direction) for all valid inputs
- [ ] edge: A and B are the same CxId → `lead_lag_secs = 0.0`, self-correlation (not stored in xterm CF)
- [ ] edge: `window_secs = 0` → no pairs found → `None`
- [ ] fail-closed: series read error → `CALYX_LOOM_SERIES_READ_ERROR`

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `temporal_xterm` CF row for `(cx_a, cx_b)` pair
- **Readback:** after ingesting 5 occurrences each for CxId-A (at t=100,200,300,400,500) and CxId-B (at t=115,215,315,415,515) with window=30s: persist the temporal cross-term JSON, run `calyx readback temporal-cross-term --artifact <temporal-cross-term.json>`, and separately read the backing CF row with `calyx readback --cf temporal_xterm --vault <vault>` or equivalent byte reader
- **Prove:** `lead_lag_secs = 15.0` (A leads B by 15s); `n_pairs = 5`; `xxd` shows the f64 bytes at the expected offset

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH42 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV

## Implementation notes

- `temporal_xterm` is a dedicated Aster CF with key `cx_a || cx_b`; `(A,B)` and
  `(B,A)` are independent rows.
- Loom stores a fixed binary `LLAG1` value: magic, `cx_a`, `cx_b`,
  big-endian `f64 lead_lag_secs`, big-endian `u64 n_pairs`, and big-endian
  `u64 proximity_window_secs`.
- `calyx_loom::temporal_cross_term` reads recurrence series through
  `SeriesStore`, maps read failures to `CALYX_LOOM_SERIES_READ_ERROR`, and
  persists only non-self, sufficient-pair results.
- FSV is driven by
  `crates/calyx-loom/tests/recurrence_cross_terms_fsv.rs`, which writes
  `temporal-cross-term.json` plus BLAKE3 manifest under the issue evidence root.
