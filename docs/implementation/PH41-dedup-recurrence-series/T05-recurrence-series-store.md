# PH41 · T05 — Recurrence series store (one event, many `t_k` occurrences; bounded, A26)

| Field | Value |
|---|---|
| **Phase** | PH41 — DedupPolicy TctCosine + Recurrence Series + Signature |
| **Stage** | S9 — Temporal & Dedup |
| **Crate** | `calyx-aster` / `calyx-loom` |
| **Files** | `crates/calyx-aster/src/recurrence.rs` (≤500), `crates/calyx-loom/src/recurrence/mod.rs` (≤500), `crates/calyx-loom/src/recurrence/series_store.rs` (≤500), `crates/calyx-cli/src/recurrence_readback.rs` (≤500) |
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

- [x] Define `Occurrence { id: OccurrenceId, t_k: EpochSecs, context: OccurrenceContext }` where `OccurrenceContext` is a small blob (≤256 bytes) of caller-provided context (e.g., session ID, source)
- [x] Define `RecurrenceSeries { cx_id: CxId, occurrences: Vec<Occurrence>, frequency: u64, cadence_secs: Option<f64>, rollup_summary: Option<RollupSummary> }`
- [x] Define `RollupSummary { oldest_t: EpochSecs, count_rolled: u64, period_estimate_secs: f64 }` — replaces oldest occurrences when rollup fires
- [x] Define `RetentionPolicy { max_occurrences: usize, max_age_secs: u64 }` — default: max_occurrences=10_000, max_age_secs=365*86400
- [x] Implement `SeriesStore::append_occurrence(cx_id: CxId, t_k: EpochSecs, context: OccurrenceContext) -> Result<OccurrenceId, CalyxError>`:
  - write occurrence to the `recurrence` CF under key `(cx_id, occ_id)` in WAL group-commit
  - increment `frequency` counter in base CF for `cx_id`
  - enforce `RetentionPolicy`: if len+1 > max_occurrences → roll up oldest 10% into `RollupSummary`, replace active rows with rolled markers
  - enforce age retention: drop occurrences older than `now - max_age_secs` → roll up
  - return new `OccurrenceId`
- [x] Implement `SeriesStore::read_series(cx_id: CxId) -> Result<RecurrenceSeries, CalyxError>`:
  - scan `recurrence` CF for `cx_id` prefix; read all occurrence rows
  - compute `cadence_secs` = median of consecutive `t_k` gaps (if ≥2 occurrences)
  - return `RecurrenceSeries` with occurrences sorted ascending by `t_k`
- [x] Implement `SeriesStore::occurrence_count(cx_id: CxId) -> Result<u64, CalyxError>` — O(1) from `frequency` field in base CF
- [x] `calyx-loom` exists from Stage 5; add a `recurrence` module and re-export it from the existing `lib.rs`

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: append 3 occurrences at t=100, 200, 300 → `read_series` returns sorted `[100, 200, 300]`; `cadence_secs = Some(100.0)`; `frequency = 3`
- [x] unit: append 1 occurrence → `cadence_secs = None` (need ≥2)
- [x] unit: `RetentionPolicy { max_occurrences: 5 }` → after 6 appends, count = 5 + rollup_summary has count_rolled=1; `frequency` still = 6
- [x] unit: age rollup: append occurrence at t=0, set retention max_age=3600 seconds, clock at t=7200 → occurrence rolled up on next append
- [x] unit: `occurrence_count` = O(1) (reads `frequency` scalar, not scan)
- [x] proptest: `frequency` always equals total appends (rolled up + retained) — never undercounts
- [x] edge: `read_series` on CxId with no occurrences → `frequency=0`, empty `occurrences`, `cadence=None`
- [x] edge: `context` blob > 256 bytes → `CALYX_RECURRENCE_CONTEXT_TOO_LARGE`
- [x] fail-closed: injected WAL append failure leaves snapshot, base frequency, and recurrence CF unchanged. #622 keeps `CALYX_DISK_PRESSURE` as the canonical PRD 18 storage-write code; no `CALYX_WAL_WRITE_ERROR` was added.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `recurrence` CF rows for a known CxId; `frequency` field in base CF
- **Readback:** `calyx readback recurrence-series <CxId>` after 5 ingests at known timestamps; print `occurrences`, `cadence_secs`, `frequency`; `xxd` the raw CF rows for `(cx_id, occ_0)` through `(cx_id, occ_4)`
- **Prove:** 5 occurrences in order; `cadence_secs` = correct median gap; `frequency = 5`; raw CF bytes contain the `t_k` values at the expected offsets

### FSV evidence captured for #383

- **Commit:** `bacf9d2aa0b6e96872d0753e23192294c771a90a`
- **Root:** `/home/croyse/calyx/data/fsv-issue383-recurrence-series-20260610-bacf9d2`
- **Artifact hash:** `recurrence-series-readback.json` BLAKE3 `130010f0aefee719fe5f2b55c2d025e6d016c34f18d3773947597ccffc46b19a`
- **Happy path:** `calyx readback recurrence-series --vault <root>/ingest/vault --cx-id 434fd701ee186cee2544d1166e0a6ea2` reads `frequency=5`, `occurrence_count=5`, `cadence_secs=100.0`, ids 0..4 at `t_k` 100, 200, 300, 400, 500. Raw `recurrence` CF readback prints row values containing those `t_k` bytes.
- **Edges:** empty CxId `e10224969b9a72b8863d4a19bc7346e6` reads zero frequency/occurrences and raw recurrence CF count 0; max-count rollup CxId `1a878fed496ac72653d03bd27a011321` reads `frequency=6`, active ids 1..5, `rollup_summary.count_rolled=1`, rolled row id 0 into 5; oversized CxId `f5e8283ed40acd977c6c8e3ce79e200e` reads zero frequency/occurrences, raw recurrence CF count 0, and persisted error `CALYX_RECURRENCE_CONTEXT_TOO_LARGE`.
- **WAL fail-closed:** #622 is FSV-backed at `/home/croyse/calyx/data/fsv-issue622-recurrence-wal-failure-20260610-bf0d380`; `CALYX_DISK_PRESSURE` is the stable code, and readback proves snapshot/base/recurrence/online/ledger state unchanged after injected WAL append failure.
- **Reclaim:** #620 is FSV-backed at `/home/croyse/calyx/data/fsv-issue620-recurrence-reclaim-20260610-209f843`; rolled rows write tombstones, recurrence compaction reclaims superseded input SSTs, prunes tombstone rows from the active compacted SST, cold-reopens with ids 4/5/6 and frequency 7, and documents WAL history retention until the general recycler.
- **Follow-ups:** #621 is closed/FSV-backed for concurrency-safe occurrence id allocation; #622 is closed/FSV-backed for the WAL failure code contract; #627 remains for CLI compaction recovery-safe naming, #628 remains for dedup undo after rolled summary FSV, and #626 remains for anchor-conflict never-merge property coverage.

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] FSV evidence (readback output / screenshot) attached to the PH41 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
