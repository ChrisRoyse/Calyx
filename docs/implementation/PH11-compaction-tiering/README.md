# PH11 — Compaction + hot/cold tiering

**Stage:** S1 — Aster storage core  ·  **Crate:** `calyx-aster`  ·
**PRD roadmap:** P0  ·  **Axioms:** A26

## Objective

Deliver background, snapshot-safe compaction (concurrent reads during SST merges),
a tiering policy that places active-slot quantized columns and base/WAL CFs on
the NVMe hot pool (`/zfs/hot/calyx`) and `*.raw` f32 sidecars, retired-slot
columns, and old panel versions on the archive HDD (`/zfs/archive/calyx`), and a
write-amp metric that stays within a target on a soak run. After PH11, the vault
can run long-term without unbounded SST file proliferation.

## Dependencies

- **Phases:** PH10 (manifest, vault open/recover), PH07 (CF routing, CF directory
  layout), PH06 (SST reader/writer, `CompactionCatalog`)
- **Provides for:** PH58 (GC reclaimers use the compaction snapshot for version
  expiry), PH35 (Ledger archive to cold tier)

## Current state (build off what exists)

`compaction/mod.rs` is already written with:
- `SstShard`, `CompactionCatalog` (atomic snapshot swap with `Arc<Vec<SstShard>>`).
- `CompactionThrottle`, `CompactionDebt` (write-amp debt meter).
- `compact_shards`: merges SSTs for one CF, writes output, returns metrics.
- `TieringPolicy` with `hot_root = /zfs/hot/calyx`, `archive_root =
  /zfs/archive/calyx`, active-slot aware, `place_cf`, `write_tiered_sst`.
- `TierWrite`, `TierPlacement`, `StorageTier`.
- `compaction/tests.rs` exists.

**What remains:**
- No background compaction thread. Need `CompactionScheduler` that runs compaction
  on a timer (or on demand when `CompactionDebt` crosses a threshold), holds a
  `CompactionCatalog`, and does not block readers.
- No integration with the vault: `AsterVault` needs a method to trigger compaction
  and to use `CompactionCatalog::pin_snapshot()` so reads do not fail during
  a compaction swap.
- No soak test verifying write-amp stays ≤ target.
- The cold-tier write uses real `/zfs/archive/calyx`; on aiwonder without ZFS
  provisioned, it should fall back to `CALYX_HOME/archive` — add a fallback path.
- Staging temp files: `write_sst` already uses `path.with_extension("sst.tmp")`
  in the same directory; confirm this is also used by tiered writes to avoid
  `EXDEV`.

## Deliverables (file plan, each ≤500 lines)

| File | Responsibility |
|---|---|
| `src/compaction/mod.rs` | `SstShard`, `CompactionCatalog`, `TieringPolicy`, `compact_shards` |
| `src/compaction/scheduler.rs` | `CompactionScheduler`: background thread, debt-metered cadence, anti-storm |
| `src/compaction/tests.rs` | Snapshot-safe concurrent test, write-amp soak, tiering placement test |

## Tasks (atomic — all must pass for the phase to be DONE)

| Card | Title | Depends |
|---|---|---|
| T01 | Compaction debt meter + throttle proptest | — |
| T02 | Snapshot-safe concurrent compaction (reads during merge) | T01, PH06 T05 |
| T03 | Tiering policy: hot/cold CF placement + staging-in-dest | T02 |
| T04 | CompactionScheduler: background thread + anti-storm | T02 |
| T05 | Write-amp soak + cold-slot physical path FSV | T03, T04 |

## FSV exit gate (the phase is DONE only when this is byte-proven on aiwonder)

Run compaction with concurrent readers on aiwonder; verify no partial reads occur
and cold slots are physically on the archive path:

```
calyx compact --vault /home/croyse/calyx/test-vault --cf slot_00.raw
ls /home/croyse/calyx/archive/cf/slot_00.raw/
calyx readback --cf base --vault /home/croyse/calyx/test-vault
```

Also: 1000-op soak with compaction running → `write_amp_milli ≤ 2000` (≤2× write
amplification). Evidence posted to PH11 GitHub issue.

## Risks / landmines

- `EXDEV` on cross-ZFS-dataset rename: staging temp files must be in the
  destination dataset. Both `write_sst` (in `sst/mod.rs`) and `TieringPolicy::
  write_tiered_sst` must create temp files in the destination CF directory.
- Compaction reads all input SST files into a `BTreeMap` in RAM; for large CFs
  (e.g., 1e7 entries × 64 B = 640 MB) this may OOM. Add a `max_input_bytes`
  throttle check and document the PH68 DiskANN path for billion-scale.
- `CompactionCatalog` atomic swap: the old `Arc<Vec<SstShard>>` is held by
  readers pinned before the swap; it must remain alive until all such snapshots
  drop. The `Arc` approach already handles this correctly.
- Anti-storm (PRD `24 §3`): if compaction runs to completion but the write rate
  is higher than the compaction rate, debt will grow unboundedly. Add a
  `max_debt_score` threshold above which the write path applies backpressure
  (`CALYX_BACKPRESSURE`).
