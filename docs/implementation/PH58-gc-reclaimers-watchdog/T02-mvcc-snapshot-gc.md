# PH58 · T02 — MVCC snapshot GC — version reclaim once no reader pins; anti-storm rate limit

| Field | Value |
|---|---|
| **Phase** | PH58 — GC reclaimers + long-reader watchdog + janitor |
| **Stage** | S13 — Resource, GC & Reliability Hardening |
| **Crate** | `calyx-aster` |
| **Files** | `crates/calyx-aster/src/gc/snapshot_gc.rs` (≤500) (extends T01 file) |
| **Depends on** | T01 (reader leases + watchdog) |
| **Axioms** | A26, A16 |
| **PRD** | `dbprdplans/24 §3`, `24 §4`, `24 §7` hazard 6 |

## Goal

Implement the MVCC version GC reclaimer: scan SST files for MVCC versions older than the
oldest-pinned sequence, mark them for deletion, and drive compaction to physically remove them.
Rate-limited to avoid write-amp storms (anti-storm rules). A version is reclaimable once no
live reader pins a sequence ≤ it (coordinated with the watchdog from T01). Also handles the
soft-delete GC 30-day window: constellations soft-deleted more than 30 days ago are purged
from base and slot column families.

## Build (checklist of concrete, code-level steps)

- [ ] Implement `SnapshotGcReclaimer::run_once(&self, watchdog: &SnapshotPinWatchdog, newest_seq: u64) -> GcResult` in `snapshot_gc.rs`:
  - calls `watchdog.oldest_pinned_seq()` to get the GC safe point; if `None` (no readers), use `newest_seq`
  - scans SST metadata for MVCC versions < safe_point; marks them reclaimable
  - calls compaction trigger with `reclaimable_versions` list; throttled (max N versions per run)
  - returns `GcResult { versions_reclaimed: usize, bytes_freed: usize }`
- [ ] Implement soft-delete GC: `SoftDeleteGcReclaimer::run_once(&self, clock: &dyn Clock) -> GcResult`:
  - scans the `soft_deleted_at` column for entries older than `now - 30d`; purges from `base` and all `slot_*` CFs
  - rate-limited: max 1000 keys per run; reschedules if more remain
  - never touches Ledger entries (append-only; PRD `24 §3`)
- [ ] Define `struct GcRateLimit { max_ops_per_run: usize, min_interval: Duration }` — applied to all reclaimers; config from env
- [ ] Implement `GcScheduler::tick(&self, clock: &dyn Clock)` — runs all reclaimers at their configured intervals; respects rate limits; lower priority than serving I/O
- [ ] Track `compaction_debt: u64` metric (number of reclaimable versions not yet collected); alert threshold
- [ ] Emit Prometheus: `calyx_gc_versions_reclaimed_total`, `calyx_gc_bytes_freed_total`, `calyx_gc_soft_deletes_purged_total`, `calyx_compaction_debt`

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: write 100 versions at seqs 1–100; advance `oldest_pinned_seq` to 50; `run_once` → exactly 49 versions reclaimed (seqs 1–49 < 50); 51 remain
- [ ] unit: all readers released (`oldest_pinned_seq == None`); `run_once` with `newest_seq=100` → all 100 versions reclaimable; rate limit caps at `max_ops_per_run`
- [ ] unit: soft-delete GC — insert 5 soft-deleted CxIds at `now - 31d`, 3 at `now - 10d`; `run_once` purges exactly 5; 3 remain; Ledger entries untouched
- [ ] unit: rate limit — `max_ops_per_run=10`; 100 reclaimable versions; `run_once` processes exactly 10; `compaction_debt` decreases by 10
- [ ] proptest: `forall pinned_seqs: Vec<u64>, version_seqs: Vec<u64>` — versions reclaimed are always < `min(pinned_seqs)`; live versions never reclaimed
- [ ] edge: `oldest_pinned_seq == newest_seq` → 0 versions reclaimable; no compaction triggered
- [ ] edge: soft-delete entry exactly at `now - 30d` boundary → purged (inclusive at 30 days)
- [ ] fail-closed: GC reclaimer panics → `GcScheduler` catches the panic via `std::panic::catch_unwind`, logs `CALYX_GC_ERROR`, reschedules; does not crash the database

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `calyx_gc_bytes_freed_total` counter, `calyx_compaction_debt` gauge, and `du -sh /hotpool/vault_*` before/after GC run on aiwonder
- **Readback:**
  ```
  calyx readback --metric gc_bytes_freed_total
  calyx readback --metric compaction_debt
  du -sh /hotpool/vault_*
  ```
- **Prove:** ingest 1e5 write+delete pairs; verify `compaction_debt > 0`; run `SnapshotGcReclaimer` and `SoftDeleteGcReclaimer` with no live readers; verify `gc_bytes_freed_total` increases and `du` output decreases. Attach readback to PH58 GitHub issue.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH58 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
