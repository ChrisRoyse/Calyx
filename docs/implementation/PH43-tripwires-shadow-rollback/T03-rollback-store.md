# PH43 - T03 - Rollback store (prior artifact + pointer swap)

| Field | Value |
|---|---|
| **Phase** | PH43 - Tripwires + Shadow-First + Reversible/Rollback |
| **Stage** | S10 - Anneal + Intelligence Objective J |
| **Crate** | `calyx-anneal` |
| **Files** | `crates/calyx-anneal/src/rollback.rs`, `crates/calyx-anneal/src/rollback_codec.rs` |
| **Depends on** | T01 (TripwireRegistry identifies what to revert) |
| **Axioms** | A14, A15 |
| **PRD** | `dbprdplans/12 section 6`, `dbprdplans/27 section 4` |

## Goal

Before any Anneal promotion, `RollbackStore` snapshots the prior artifact under a
monotonic `ChangeId`. If a promoted candidate must be reverted,
`rollback(change_id)` restores the prior live pointer with one atomic state
mutation and no artifact data movement.

## Implementation

- [x] `ChangeId(u64)` is allocated from `Clock::now()`, a seeded counter bucket,
  and the recovered max durable change id so successful ids are monotonic and not
  reused.
- [x] `ArtifactSnapshot` records `change_id`, `ArtifactKey`, prior pointer,
  candidate pointer, logical timestamp, description, and promoted/reverted/
  committed flags.
- [x] `ArtifactPtr` covers config-cache key hashes, HNSW graph paths, and
  quant-level record hashes.
- [x] `RollbackStore` keeps `snapshots` and `live_ptrs` behind a `RwLock`; this is
  the data-race-free swap primitive used instead of adding an ArcSwap dependency.
- [x] `install_live_ptr`, `prepare`, `promote`, `rollback`, and `commit` write
  durable rows first, then update memory, so storage failure leaves no partial
  snapshot/live-pointer mutation.
- [x] Aster has a real `anneal_rollback` column family with WAL tag 9, durable
  recovery parsing, compaction scan parsing, and generic CLI CF readback support.
- [x] `AsterVault::write_cf_batch` exposes a narrow WAL-backed raw CF batch API
  used by `AsterRollbackStorage`.
- [x] Snapshot/live rows use a deterministic binary codec:
  - snapshot key: `change:` + big-endian `ChangeId`
  - live key: `live:` + artifact-key tag/hash
  - snapshot value magic: `ARS1`
  - live value magic: `ARL1`

## Tests

- [x] Unit: `prepare` -> `promote` -> `rollback` restores the prior live pointer
  bytes exactly.
- [x] Unit: rollback after commit returns `CALYX_ANNEAL_CHANGE_COMMITTED`.
- [x] Unit/edge: unknown change and empty store return
  `CALYX_ANNEAL_UNKNOWN_CHANGE_ID`.
- [x] Unit/edge: concurrent promote and rollback on different keys succeed
  independently.
- [x] Fail-closed: injected storage/WAL failure during `prepare` propagates the
  upstream `CALYX_ASTER_WAL_SYNC` code and records no snapshot row.
- [x] Proptest: operation sequences never leave the live pointer undefined or
  outside the known prior/candidate set.
- [x] Ignored FSV trigger: `rollback_fsv.rs` creates durable Aster
  `anneal_rollback` rows and byte artifacts under `CALYX_ISSUE396_FSV_ROOT`.

## FSV

Source of truth:

- Aster CF: `<vault>/cf/anneal_rollback`
- WAL: `<vault>/wal`
- FSV root: `/home/croyse/calyx/data/fsv-issue396-rollback-<timestamp>`

Readback paths:

- Generic CLI: `calyx readback --cf anneal_rollback --vault <vault>`
- Direct byte files emitted by the ignored FSV trigger:
  - `live-before.bin`
  - `snapshot-after-prepare.bin`
  - `live-after-promote.bin`
  - `snapshot-after-rollback.bin`
  - `rollback-readback.json`
  - `BLAKE3SUMS.txt`

Required proof:

1. Install prior live pointer.
2. Prepare a candidate and read the snapshot row.
3. Promote and read the live row as the candidate pointer.
4. Roll back and read the live row as the prior pointer.
5. Reopen the durable vault and read the same prior pointer from
   `anneal_rollback`.
6. Exercise at least three edges with before/after CF scans: unknown change id,
   missing live pointer, and rollback of a committed change.

## Status

Done. #396 closed with aiwonder gates plus manual byte readback evidence at
`/home/croyse/calyx/data/fsv-issue396-rollback-20260610-2314`.
