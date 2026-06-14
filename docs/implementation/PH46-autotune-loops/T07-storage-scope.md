# PH46 T07 - Aster storage scope tuner

| Field | Value |
|---|---|
| **Phase** | PH46 - Autotune Loops |
| **Stage** | S10 - Anneal + Intelligence Objective J |
| **Crate** | `calyx-anneal`, `calyx-cli` |
| **Files** | `crates/calyx-anneal/src/tune/scope_storage.rs`, `crates/calyx-cli/src/anneal_autotune_report.rs` |
| **Depends on** | T01 (ConfigBandit), Aster compaction/tiering |
| **PRD** | `dbprdplans/12 A-4` storage autotune layer |

## Goal

Add the missing storage autotune layer for Aster. The scope tunes compaction
cadence, hot/cold tier thresholds, codebook refresh cadence, and prefetch size
using the same PH46 primitives as Forge/Index/Loom: `AutotuneCache` rows,
`anneal_bandit` CF rows, and Anneal Ledger promotions.

## Build

- [x] `StorageConfig` captures compaction interval, debt trigger, write-amp cap,
  hot-tier hit threshold, cold-tier idle threshold, codebook refresh cadence, and
  prefetch bytes.
- [x] `StorageScopeTuner` owns Thompson candidates per storage shape key.
- [x] Promotion requires lower p99 read latency and no regression in write amp,
  cache miss rate, hot-tier hit rate, codebook staleness, or prefetch hit rate.
- [x] Promotion writes `AutotuneCache`, `anneal_bandit`, and Ledger rows with
  artifact ids prefixed `storage:`.
- [x] `calyx anneal autotune-report --scope storage` prints storage cache rows,
  per-shape bandit readback, and recent storage Ledger promotions.

## FSV

- **SoT:** persisted storage cache JSON, Aster `anneal_bandit` CF, Aster Ledger CF,
  and vault WAL bytes.
- **Happy path:** synthetic storage workload promotes a shorter compaction
  interval and larger prefetch config after repeated measured wins.
- **Edges:** write-amp regression does not promote; invalid per-mille metrics
  fail closed with `CALYX_STORAGE_SCOPE_INVALID_CONFIG`; invalid prefetch byte
  alignment fails closed with `CALYX_STORAGE_SCOPE_INVALID_CONFIG`.
- **Readback:** ignored test `scope_storage_fsv` writes
  `storage-scope-readback.json`; manual issue FSV must still read the physical
  cache, CF, Ledger, and WAL bytes on aiwonder.
