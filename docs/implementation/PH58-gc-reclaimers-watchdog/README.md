# PH58 — GC reclaimers + long-reader watchdog + janitor

**Stage:** S13 — Resource, GC & Reliability Hardening  ·  **Crate:** `calyx-aster`, `calyx-anneal`  ·
**PRD roadmap:** RESOURCE  ·  **Axioms:** A26

## Objective

Reclaim every kind of database garbage; defeat the classic MVCC long-reader version pile-up;
keep the operational footprint from accumulating. This phase wires bounded background
reclaimers for every garbage category: LSM tombstones/compaction, soft-delete GC (30-day
window), snapshot GC (version reclaim), WAL recycler, ANN tombstones, lazy-xterm LRU, retired-
lens columns, panel/codebook version GC, orphan reconciler. Reader leases with a max age and a
snapshot-pin watchdog that aborts long readers (`CALYX_READER_LEASE_EXPIRED`) so their pinned
MVCC versions can be GC'd. The build-artifact/log/temp/dataset janitor keeps the single-NVMe
`hotpool` from filling (PRD `24 §7b`). Anti-storm rate limits throughout. Cross-cutting
hardening from Stage 1, finalized here.

## Dependencies

- **Phases:** PH11 (compaction + tiering — the compaction reclaimer builds on this), PH08
  (MVCC snapshot reads — reader leases bound snapshot lifetimes), PH43 (Anneal tripwires —
  janitor is Anneal-managed)
- **Provides for:** PH59 (hazards 1–6, 17, 21 FSV driven from these reclaimers)

## Current state (build off what exists)

`calyx-aster` has compaction (PH11), MVCC (PH08), WAL (PH05), and SSTable (PH06) from
Stages 1–2. None have GC rate limits, reader lease enforcement, or a watchdog. The WAL
recycles segments internally but has no anti-storm guard. No janitor exists. `calyx-anneal`
is stubbed (Anneal background task infrastructure from PH43 assumed complete by time PH58
runs). Single-NVMe `hotpool` has no redundancy; buildup = an outage.

T01 (#481) now adds the reader-lease watchdog in `calyx-aster::gc::snapshot_gc`, wires
expired read pins through `CALYX_READER_LEASE_EXPIRED`, and exposes the
`AsterVault::snapshot_gc_tick(max_gap_seqs)` hook for the PH58 scheduler. The general PH58
GC scheduler is still owned by T02+; until it exists, resource-status and explicit ticks
refresh expired reader pins. Evidence root:
`/home/croyse/calyx/data/fsv-issue481-ph58-reader-leases-20260614T221758Z`.

## Deliverables (file plan, each ≤500 lines)

| File | Responsibility |
|---|---|
| `crates/calyx-aster/src/gc/compaction_gc.rs` | Adaptive throttled compaction + tombstone/soft-delete GC |
| `crates/calyx-aster/src/gc/snapshot_gc.rs` | MVCC version GC; snapshot-pin watchdog; `CALYX_READER_LEASE_EXPIRED` |
| `crates/calyx-aster/src/gc/wal_recycler.rs` | WAL segment recycle once durable; anti-storm rate limit |
| `crates/calyx-aster/src/gc/ann_gc.rs` | ANN graph tombstone purge; 10-min adaptive rebuild |
| `crates/calyx-aster/src/gc/orphan_reconciler.rs` | Orphan slot/index scan → repair/purge |
| `crates/calyx-aster/src/gc/panel_version_gc.rs` | Retired-lens columns + panel/codebook version GC |
| `crates/calyx-anneal/src/janitor.rs` | Build-artifact/log/temp/dataset janitor; disk-pressure guard; `CALYX_DISK_PRESSURE` |
| `crates/calyx-aster/src/gc/mod.rs` | Re-exports + `GcStats` + rate-limit configs |

## Tasks (atomic — all must pass for the phase to be DONE)

| Card | Title | Depends |
|---|---|---|
| T01 | Reader leases + snapshot-pin watchdog — `CALYX_READER_LEASE_EXPIRED`, version reclaim | PH08 |
| T02 | MVCC snapshot GC — version reclaim once no reader pins; anti-storm rate limit | T01 |
| T03 | Compaction GC + tombstone/soft-delete reclaimer — 30-day window, throttled | PH11 |
| T04 | WAL recycler anti-storm + ANN tombstone purge | T02, T03 |
| T05 | Orphan reconciler + panel/codebook version GC | T04 |
| T06 | Janitor — build artifacts, logs, temp files, datasets; disk-pressure guard | PH43 (Anneal infra) |
| T07 | GC FSV: long reader aborted → version GC'd, disk flat; tombstone ratio bounded | T01–T06 |

## FSV exit gate (the phase is DONE only when this is byte-proven on aiwonder)

Three proofs on aiwonder:

1. Long-reader abort → version reclaim:
   ```
   calyx readback --metric oldest_pinned_seq_gap
   calyx readback --metric reader_lease_expired_total
   ```
   Start a long scan; let its lease expire; `CALYX_READER_LEASE_EXPIRED` fires; read
   `oldest_pinned_seq_gap` — must decrease (version reclaimed); `df -h /hotpool` flat.

2. Delete-heavy workload → tombstone ratio bounded:
   ```
   calyx readback --metric tombstone_ratio
   ```
   Run 1e6 deletes; tombstone_ratio ≤ configured max (e.g., 0.5) after GC sweep.

3. Logs/build artifacts bounded:
   ```
   du -sh $CALYX_HOME/logs $CALYX_HOME/target
   ```
   After janitor sweep, bounded within configured TTL/size caps.

Evidence (all three readbacks) attached to PH58 GitHub issue.

## Risks / landmines

- **Reader lease abort races with ongoing query:** the abort must complete the current response
  (or return partial results with `CALYX_READER_LEASE_EXPIRED` in the response), not panic
- **Compaction storm during GC:** throttle compaction to < 20% of disk I/O; `compaction_debt`
  metric alerts before stall; GC rate-limit interacts with backpressure (PH56 T04)
- **FoundationDB 5-second discipline:** reader leases default to 5 s; long analytical scans
  use bounded-staleness snapshots (checkpoint read, not live pinning)
- **`EXDEV` on janitor temp file cleanup:** janitor must use `TempFile::in_dataset` (PH56 T06)
  for any staging; if a temp file must cross ZFS dataset boundary, copy then delete
- **Ledger is never deleted:** archival moves to cold/restic; `panel_version_gc.rs` must not
  touch Ledger entries — only lens/panel/codebook data
