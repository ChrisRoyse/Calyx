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

## Status — DONE ✅ (Stage 1; FSV-signed-off 2026-06-07; durable tiering post-sweep FSV 2026-06-08)

Shipped in `calyx-aster`:
- `compaction/mod.rs` + `compaction/tiering.rs` — `CompactionDebt::measure` (score), `CompactionThrottle` (`max_input_bytes`), snapshot-safe `CompactionCatalog` (atomic `Arc<Vec<SstShard>>` swap; pinned readers survive), `TieringPolicy::place_cf`/`is_cold` (base/ledger/anchors hot; `*.raw`/retired/old-panel cold), `write_tiered_sst` (stages in destination dir; `aiwonder()` → `/zfs/hot|archive/calyx` with `CALYX_HOME` fallback), `CompactionScheduler` (background thread, debt trigger, write-amp backoff, `FIXME(PH46)` Anneal hook).
- `vault/durable.rs`, `cf/router.rs`, `compaction/scan.rs`, `vault/compaction_bridge.rs` — `VaultOptions::tiering_policy` wires the policy into normal durable checkpoint SSTs, MVCC router flush SSTs, manifest recovery scans across tier roots, vault compaction catalogs, one-shot compaction output, and background scheduler output.
- CLI: `tier`, `compact`, `compact-watch`, `soak`. FSV-proven: compacted base SST written; `slot_00` hot + `slot_00.raw` archive placement; `xxd` showed `CXS1`.

FSV evidence: GitHub issue #23 (`[CONTEXT] You are here`); Stage-1 evidence root `/home/croyse/calyx/data/fsv-stage1-exit-20260607105216`; durable tiering follow-up root `/home/croyse/calyx/data/fsv-issue295-tiered-vault-20260608`.

### Resolved follow-ups
1. `CompactionScheduler`/`CompactionCatalog` are wired into `AsterVault`; #295 extends that wiring through hot/archive tier roots so cold CFs are not duplicated under the hot vault root.
2. The compaction-debt meter has generated coverage in `compaction/tests.rs::compaction_debt_matches_scaled_pending_bytes`.

## Deliverables (file plan, each ≤500 lines)

| File | Responsibility |
|---|---|
| `src/compaction/mod.rs` | `SstShard`, `CompactionCatalog`, `compact_shards`, scheduler facade |
| `src/compaction/tiering.rs` | `TieringPolicy`, placement, destination-dataset SST writer |
| `src/compaction/scan.rs` | vault/tier SST catalog discovery |
| `src/vault/durable.rs` / `src/cf/router.rs` | normal durable and router flushes routed through `TieringPolicy` |
| `src/vault/compaction_bridge.rs` | vault compaction catalog/output/scheduler bridge |
| `src/compaction/tests.rs` / `src/vault/compaction_tests.rs` | snapshot-safe, debt, tier placement, durable tier readback |

## Tasks (atomic — all must pass for the phase to be DONE)

| Card | Title | Depends |
|---|---|---|
| T01 | Compaction debt meter + throttle proptest | — |
| T02 | Snapshot-safe concurrent compaction (reads during merge) | T01, PH06 T05 |
| T03 | Tiering policy: hot/cold CF placement + staging-in-dest | T02 |
| T04 | CompactionScheduler: background thread + anti-storm | T02 |
| T05 | Write-amp soak + cold-slot physical path FSV | T03, T04 |

## FSV exit gate (the phase is DONE only when this is byte-proven on aiwonder)

> ✅ **Achieved** — byte-proven on aiwonder; evidence in GitHub issue #23 (Stage-1 FSV root `/home/croyse/calyx/data/fsv-stage1-exit-20260607105216`) and #295 (`/home/croyse/calyx/data/fsv-issue295-tiered-vault-20260608`).

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
