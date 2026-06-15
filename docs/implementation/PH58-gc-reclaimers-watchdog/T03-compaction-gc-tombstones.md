# PH58 · T03 — Compaction GC + tombstone/soft-delete reclaimer — 30-day window, throttled

| Field | Value |
|---|---|
| **Phase** | PH58 — GC reclaimers + long-reader watchdog + janitor |
| **Stage** | S13 — Resource, GC & Reliability Hardening |
| **Crate** | `calyx-aster` |
| **Files** | `crates/calyx-aster/src/gc/compaction_gc.rs` (≤500) |
| **Depends on** | T02 (snapshot GC rate-limit pattern) · PH11 (compaction exists) |
| **Axioms** | A26, A16 |
| **PRD** | `dbprdplans/24 §3`, `24 §7` hazard 1, hazard 2, hazard 3 |

## Goal

Instrument the existing compaction (PH11) with adaptive throttling, a compaction-debt metric,
and an anti-storm rate limiter so compaction never starves serving (hazard 1: write-amp storm;
hazard 2: flush stall; hazard 3: tombstone buildup). Also implement the tombstone ratio monitor:
if the ratio of tombstone keys to live keys exceeds 0.5, trigger an emergency compaction pass
(rate-limited). This reclaimer is the primary mechanism for physically removing deleted data
and overwritten versions from SST files.

## Build (checklist of concrete, code-level steps)

- [ ] Define `struct CompactionGcReclaimer { max_io_fraction: f64, debt_metric: Arc<AtomicU64>, compaction_debt_alert_threshold: u64 }` in `compaction_gc.rs`
- [ ] Implement `CompactionGcReclaimer::estimate_write_amp(&self) -> f64` — reads compaction stats from the manifest (bytes_written_by_compaction / bytes_written_by_flush); returns current write amplification
- [ ] Implement `CompactionGcReclaimer::tombstone_ratio(&self) -> f64` — scans SST metadata for tombstone key count vs live key count; returns ratio
- [ ] Implement `CompactionGcReclaimer::maybe_trigger(&self, disk_io_available_fraction: f64)` — if `tombstone_ratio > 0.5` AND `disk_io_available_fraction > max_io_fraction`, trigger a partial compaction pass; else skip (anti-storm: do not starve serving)
- [ ] Implement `CompactionGcReclaimer::adaptive_cadence(&self)` — integrate with Anneal: if `compaction_debt > alert_threshold` and serving p99 is below SLO, increase compaction aggressiveness; else reduce — PRD `24 §3` "adaptive cadence (Anneal)"
- [ ] Emit Prometheus: `calyx_tombstone_ratio`, `calyx_write_amp`, `calyx_compaction_debt`, `calyx_compaction_debt_alert` (binary flag)
- [ ] Anti-storm: `CompactionThrottle` struct with token-bucket rate limiter; compaction consumes tokens proportional to bytes compacted; bucket refills at `max_io_fraction × disk_bw`
- [ ] Implement `CompactionGcReclaimer::run_once(&self) -> GcResult` — one throttled compaction pass; returns `bytes_compacted`, `tombstones_removed`, `write_amp_delta`

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: inject 100 tombstones and 100 live keys into SST metadata mock; `tombstone_ratio()` returns `0.5`; inject 200 tombstones, 100 live → `0.667`
- [ ] unit: `maybe_trigger` with `tombstone_ratio=0.6` and `io_available=0.5` (> `max_io_fraction=0.2`) → triggers; with `io_available=0.1` → skips
- [ ] unit: `CompactionThrottle` token bucket — request 1 MiB; if tokens available, proceed; if not, wait; `run_once` never exceeds `max_io_fraction` I/O budget over a 1-s window
- [ ] unit: `estimate_write_amp()` with `bytes_written_by_compaction=4 GiB` and `bytes_written_by_flush=1 GiB` → returns `4.0`
- [ ] proptest: `forall tombstone_counts, live_counts` — `tombstone_ratio` is always `tombstones / (tombstones + live)` ∈ [0.0, 1.0]
- [ ] edge: all-tombstone SST (live=0) → `tombstone_ratio == 1.0`; `maybe_trigger` fires; compaction removes all tombstones
- [ ] edge: zero tombstones → `tombstone_ratio == 0.0`; `maybe_trigger` does not fire (no compaction needed)
- [ ] fail-closed: compaction I/O error → `GcResult { error: Some(CALYX_IO_ERROR) }`; debt metric NOT decremented (debt remains accurate); next run retries

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `calyx_tombstone_ratio` gauge and `calyx_write_amp` gauge on aiwonder during a delete-heavy workload
- **Readback:**
  ```
  calyx readback --metric tombstone_ratio
  calyx readback --metric write_amp
  calyx readback --metric compaction_debt
  ```
- **Prove:** run 1e6 deletes on aiwonder; `tombstone_ratio` rises above 0.5; `compaction_gc` triggers; after GC sweep, `tombstone_ratio` drops below 0.1; `write_amp` ≤ configured target; serving p99 unchanged (anti-storm worked). Attach readback output to PH58 GitHub issue.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH58 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
