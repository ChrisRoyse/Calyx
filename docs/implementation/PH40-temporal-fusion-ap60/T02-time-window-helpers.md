# PH40 · T02 — TimeWindow helpers (`last_hours` / `last_days`)

| Field | Value |
|---|---|
| **Phase** | PH40 — Temporal Fusion + AP-60 Post-Retrieval Boost |
| **Stage** | S9 — Temporal & Dedup |
| **Crate** | `calyx-sextant` |
| **Files** | `crates/calyx-sextant/src/temporal/window.rs` (≤500) |
| **Depends on** | T01 (this phase) |
| **Axioms** | A27 |
| **PRD** | `dbprdplans/25 §3`, `dbprdplans/25 §8` |

## Goal

Implement `TimeWindow` — a half-open interval `[start, end)` in UTC epoch
seconds — plus the `last_hours(n)` and `last_days(n)` constructors that compute
the window relative to an injected `Clock` (never `SystemTime::now()`). Also
implement `TimeWindow::contains(event_time_secs)` and the filter helper
`filter_hits_by_window(hits, window)` that removes hits outside the window
without reordering the remaining hits (AP-60: in-window ranking is undistorted).

## Build (checklist of concrete, code-level steps)

- [x] Define `TimeWindow { start_secs: i64, end_secs: i64 }` with invariant `start_secs < end_secs` → `CALYX_TEMPORAL_INVALID_WINDOW` on violation
- [x] Implement `TimeWindow::last_hours(n: u64, clock: &dyn Clock) -> Result<TimeWindow, CalyxError>` — `end = clock.now_secs()`, `start = end − n*3600`; `n == 0` → `CALYX_TEMPORAL_INVALID_WINDOW`
- [x] Implement `TimeWindow::last_days(n: u64, clock: &dyn Clock) -> Result<TimeWindow, CalyxError>` — `end = clock.now_secs()`, `start = end − n*86400`; `n == 0` → `CALYX_TEMPORAL_INVALID_WINDOW`
- [x] Implement `TimeWindow::contains(&self, event_time_secs: i64) -> bool` — true iff `start_secs <= event_time_secs < end_secs`
- [x] Implement `TimeWindow::all()` → open window (no filtering)
- [x] Implement `filter_hits_by_window(hits: Vec<Hit>, window: &TimeWindow) -> Vec<Hit>` — drops hits whose `event_time_secs` is outside the window; preserves original order of retained hits (no reranking inside this function)
- [x] `Clock` trait: `fn now_secs(&self) -> i64`; provide `SystemClock` (real time, not used in logic) and `FixedClock { secs: i64 }` (test injection)
- [x] All types `serde::{Serialize, Deserialize}` + `Clone` + `Debug`

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: `FixedClock { secs: 1_000_000 }` → `last_hours(1)` → `start = 996_400`, `end = 1_000_000`
- [x] unit: `FixedClock { secs: 1_000_000 }` → `last_days(1)` → `start = 913_600`, `end = 1_000_000`
- [x] unit: `TimeWindow { start: 100, end: 200 }.contains(150)` → true; `contains(200)` → false (half-open); `contains(99)` → false
- [x] unit: `filter_hits_by_window` with 5 hits at times [50, 120, 170, 250, 300], window [100,200) → retains hits at 120 and 170 in original order
- [x] proptest: `filter_hits_by_window` is a subset: `result ⊆ input` (every retained hit was in original)
- [x] edge: `last_hours(0)` → `CALYX_TEMPORAL_INVALID_WINDOW`
- [x] edge: `TimeWindow { start: 200, end: 100 }` → `CALYX_TEMPORAL_INVALID_WINDOW`
- [x] fail-closed: `last_hours(u64::MAX)` arithmetic overflow → `CALYX_TEMPORAL_INVALID_WINDOW` (not a panic)

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `filter_hits_by_window` output read back from
  `temporal-window-readback.json` under the aiwonder FSV root
- **Readback:** `cat`/`xxd` of `temporal-window-input.json` and
  `temporal-window-readback.json` at
  `/home/croyse/calyx/data/fsv-issue374-time-window-20260609-d872c7c`
- **Prove:** exactly two hits present (the two in-window constellations); the out-of-window constellation is absent; order of the two retained hits matches the pre-filter ranking order

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] FSV evidence (readback output / screenshot) attached to GitHub issue #374
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
