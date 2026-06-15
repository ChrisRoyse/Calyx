# PH11 · T04 — CompactionScheduler: background thread + anti-storm

| Field | Value |
|---|---|
| **Phase** | PH11 — Compaction + hot/cold tiering |
| **Stage** | S1 — Aster storage core |
| **Crate** | `calyx-aster` |
| **Files** | `crates/calyx-aster/src/compaction/scheduler.rs` (≤500) |
| **Depends on** | T02 (CompactionCatalog), T03 (TieringPolicy) |
| **Axioms** | A26 |
| **PRD** | `dbprdplans/04 §6`, `dbprdplans/24 §3` (anti-storm) |

## Goal

Implement `CompactionScheduler`: a background thread that wakes periodically
(default: 10-second interval, matching ContextGraph HNSW compaction cadence),
checks `CompactionDebt` for each CF, and runs `compact_cf` if debt exceeds the
trigger threshold. Implements anti-storm: if a previous compaction run's
write-amp exceeded 2× (score_milli > 2000), back off the cadence by 2×. The
Anneal adaptive hook (PH46) plugs in later; for now the scheduler is
configurable.

## Build (checklist of concrete, code-level steps)

- [x] Define `CompactionSchedulerOptions`:
  - `interval_ms: u64` (default 10_000)
  - `debt_trigger_score_milli: u64` (default 1000 = 1× write-amp)
  - `max_write_amp_milli: u64` (default 2000 = 2× write-amp)
  - `backoff_factor: u64` (default 2)
  - `max_interval_ms: u64` (default 60_000)
- [x] Define `CompactionScheduler`:
  - `catalog: Arc<CompactionCatalog>`
  - `options: CompactionSchedulerOptions`
  - `thread: JoinHandle<()>`
- [x] `CompactionScheduler::start(catalog, options) -> Self`: spawns a thread that
  loops: sleep `interval_ms`, check debt for each CF, compact if debt ≥ trigger,
  adjust interval on high write-amp.
- [x] `CompactionScheduler::stop(self)` signals the thread to exit and joins.
- [x] The scheduler thread does NOT hold any lock during the `compact_cf` call
  (the catalog's own lock is sufficient).
- [x] Use `Clock` trait for the sleep timer (injectable for tests — a
  `ManualClock` that returns instants on demand).
- [x] Add `// FIXME(PH46): replace fixed cadence with Anneal adaptive hook` comment.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit (with injected clock): scheduler fires after `interval_ms`; debt above
  trigger → `compact_cf` called; debt below trigger → skipped.
- [x] unit: write-amp > `max_write_amp_milli` → interval doubled; write-amp ≤ →
  interval unchanged.
- [x] unit: `stop()` exits the thread cleanly without panic.
- [x] edge (≥3): (1) scheduler with 0 CFs → no compaction, no panic; (2) interval
  backed off to `max_interval_ms` and stays there; (3) compaction error during
  scheduler run → logged, scheduler continues (does not exit).
- [x] fail-closed: compaction error in background → does not propagate to callers
  (background error → log + continue); catalog unchanged after failed compact.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** Number of SST files in a CF directory before and after a scheduler run.
- **Readback:**
  ```
  calyx compact-watch --vault /home/croyse/calyx/test-vault --duration 30s
  ls /home/croyse/calyx/test-vault/cf/base/
  ```
- **Prove:** After 30 seconds with continuous ingestion, the base CF directory
  has fewer SST files than the input count (compaction ran); `calyx readback`
  still returns all constellations. Screenshot posted to PH11 GitHub issue.

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] FSV evidence (readback output / screenshot) attached to the PH11 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
