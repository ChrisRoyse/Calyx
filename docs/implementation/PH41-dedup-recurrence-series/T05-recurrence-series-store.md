# PH41 · T05 — Recurrence series store (one event, many `t_k` occurrences; bounded, A26)

| Field | Value |
|---|---|
| **Phase** | PH41 — DedupPolicy TctCosine + Recurrence Series + Signature |
| **Stage** | S9 — Temporal & Dedup |
| **Crate** | `calyx-loom` |
| **Files** | `crates/calyx-loom/src/recurrence/mod.rs` (≤500), `crates/calyx-loom/src/recurrence/series_store.rs` (≤500) |
| **Depends on** | T04 (this phase) · PH09 (aster CF infrastructure) |
| **Axioms** | A28, A26 |
| **PRD** | `dbprdplans/25 §4`, `dbprdplans/25 §4b` |

## Goal

Implement the recurrence series store in `calyx-loom`: one constellation stores
many timestamped occurrences `(t_k, context)` in a dedicated CF, forming the
grounded frequency count that is the most honest signal in the system (A2/A29).
The store is bounded: a max occurrence count and a retention window are enforced
(A26); old occurrences are rolled up into a summary scalar rather than kept
unbounded. A cadence scalar (median inter-occurrence gap in seconds) is derived
on read.

## Build (checklist of concrete, code-level steps)

- [ ] Define `Occurrence { id: OccurrenceId, t_k: EpochSecs, context: OccurrenceContext }` where `OccurrenceContext` is a small blob (≤256 bytes) of caller-provided context (e.g., session ID, source)
- [ ] Define `RecurrenceSeries { cx_id: CxId, occurrences: Vec<Occurrence>, frequency: u64, cadence_secs: Option<f64>, rollup_summary: Option<RollupSummary> }`
- [ ] Define `RollupSummary { oldest_t: EpochSecs, count_rolled: u64, period_estimate_secs: f64 }` — replaces oldest occurrences when rollup fires
- [ ] Define `RetentionPolicy { max_occurrences: usize, max_age_secs: u64 }` — default: max_occurrences=10_000, max_age_secs=365*86400
- [ ] Implement `SeriesStore::append_occurrence(cx_id: CxId, t_k: EpochSecs, context: OccurrenceContext) -> Result<OccurrenceId, CalyxError>`:
  - write occurrence to the `recurrence` CF under key `(cx_id, occ_id)` in WAL group-commit
  - increment `frequency` counter in base CF for `cx_id`
  - enforce `RetentionPolicy`: if len+1 > max_occurrences → roll up oldest 10% into `RollupSummary`, delete those rows
  - enforce age retention: drop occurrences older than `now - max_age_secs` → roll up
  - return new `OccurrenceId`
- [ ] Implement `SeriesStore::read_series(cx_id: CxId) -> Result<RecurrenceSeries, CalyxError>`:
  - scan `recurrence` CF for `cx_id` prefix; read all occurrence rows
  - compute `cadence_secs` = median of consecutive `t_k` gaps (if ≥2 occurrences)
  - return `RecurrenceSeries` with occurrences sorted ascending by `t_k`
- [ ] Implement `SeriesStore::occurrence_count(cx_id: CxId) -> Result<u64, CalyxError>` — O(1) from `frequency` field in base CF
- [ ] `calyx-loom` exists from Stage 5; add a `recurrence` module and re-export it from the existing `lib.rs`

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: append 3 occurrences at t=100, 200, 300 → `read_series` returns sorted `[100, 200, 300]`; `cadence_secs = Some(100.0)`; `frequency = 3`
- [ ] unit: append 1 occurrence → `cadence_secs = None` (need ≥2)
- [ ] unit: `RetentionPolicy { max_occurrences: 5 }` → after 6 appends, count = 5 + rollup_summary has count_rolled=1; `frequency` still = 6
- [ ] unit: age rollup: append occurrence at t=0, set retention max_age=3600 seconds, clock at t=7200 → occurrence rolled up on next append
- [ ] unit: `occurrence_count` = O(1) (reads `frequency` scalar, not scan)
- [ ] proptest: `frequency` always equals total appends (rolled up + retained) — never undercounts
- [ ] edge: `read_series` on CxId with no occurrences → `frequency=0`, empty `occurrences`, `cadence=None`
- [ ] edge: `context` blob > 256 bytes → `CALYX_RECURRENCE_CONTEXT_TOO_LARGE`
- [ ] fail-closed: WAL write fails on append → `CALYX_WAL_WRITE_ERROR`; occurrence not committed

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `recurrence` CF rows for a known CxId; `frequency` field in base CF
- **Readback:** `calyx readback recurrence-series <CxId>` after 5 ingests at known timestamps; print `occurrences`, `cadence_secs`, `frequency`; `xxd` the raw CF rows for `(cx_id, occ_0)` through `(cx_id, occ_4)`
- **Prove:** 5 occurrences in order; `cadence_secs` = correct median gap; `frequency = 5`; raw CF bytes contain the `t_k` values at the expected offsets

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH41 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
