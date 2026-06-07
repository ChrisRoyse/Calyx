# PH08 — MVCC sequence numbers + snapshot reads

**Stage:** S1 — Aster storage core  ·  **Crate:** `calyx-aster`  ·
**PRD roadmap:** P0  ·  **Axioms:** A26

## Objective

Wire the vault-wide MVCC sequence allocator into the on-disk CF router so that
every write advances the sequence exactly once, a reader can pin a seq and read a
consistent snapshot across all CFs at that seq (no partial-constellation
visibility), and bounded-staleness reads are supported via `Freshness::StaleOk`.
The reader-lease watchdog scaffold is placed here (full lease expiry in PH58).

## Dependencies

- **Phases:** PH07 (CF router on disk), PH04 (Seq, Clock trait, CalyxError)
- **Provides for:** PH09 (vault put/get uses MVCC-seq write groups),
  PH10 (recovery restores seq from WAL), PH58 (watchdog evicts expired leases)

## Status — DONE ✅ (Stage 1; FSV-signed-off 2026-06-07, commit 8dcddaa)

Shipped in `calyx-aster`:
- `mvcc/lease.rs` — `SeqAllocator` (atomic, monotonic; `set_start_seq` fails closed after first alloc); `ReaderLease` expiry → `CALYX_READER_LEASE_EXPIRED`.
- `mvcc/store.rs` — `VersionedCfStore::commit_batch` allocates ONE seq under the write lock for the whole group, writes through `router.put`; `read_batch` resolves all CFs at one pinned seq; `Freshness::{FreshDerived,StaleOk}` → `CALYX_STALE_DERIVED`.
- `mvcc/mod.rs` + `mvcc/tests/{allocator,freshness,isolation,router_bridge}.rs` — incl. `concurrent_reader_never_observes_partial_constellation` (1000 iters).

FSV evidence: GitHub issue #23 (`[CONTEXT] You are here`); Stage-1 evidence root `/home/croyse/calyx/data/fsv-stage1-exit-20260607105216`.

## Deliverables (file plan, each ≤500 lines)

| File | Responsibility |
|---|---|
| `src/mvcc/mod.rs` | Re-exports, bridge wiring comment |
| `src/mvcc/store.rs` | `VersionedCfStore` + `CfRouter` write bridge |
| `src/mvcc/lease.rs` | `SeqAllocator`, `ReaderLease`, `Snapshot`, `Freshness` |
| `src/mvcc/tests.rs` | Concurrency test, proptest, snapshot isolation |

## Tasks (atomic — all must pass for the phase to be DONE)

| Card | Title | Depends |
|---|---|---|
| T01 | SeqAllocator monotonicity + proptest | — |
| T02 | Snapshot isolation: concurrent writer+reader no partial read | T01 |
| T03 | Freshness / bounded-staleness reads | T01 |
| T04 | MVCC+CfRouter write bridge: disk persistence under seq | T01, PH07 T03 |

## FSV exit gate (the phase is DONE only when this is byte-proven on aiwonder)

> ✅ **Achieved** — byte-proven on aiwonder; evidence in GitHub issue #23 (Stage-1 FSV root `/home/croyse/calyx/data/fsv-stage1-exit-20260607105216`).

Run a concurrent writer+reader on aiwonder: writer puts N constellations; reader
pins seq S mid-write and reads `base` + `slot_00` at seq S; assert reader sees
either both rows or neither for each constellation, never one without the other.

```
calyx mvcc-drill --vault /home/croyse/calyx/test-vault --concurrent
xxd /home/croyse/calyx/test-vault/cf/base/000001.sst | head -4
```

Evidence (terminal output showing seq-pinned reads + SST bytes) posted to PH08
GitHub issue.

## Risks / landmines

- The in-memory `VersionedCfStore` grows unboundedly (every old version retained).
  PH58 adds GC; for now, document the PH58 dependency and add a `FIXME` comment.
- `commit_batch` must be atomic with the seq advance: the seq must not be visible
  to readers until all rows in the batch are inserted. The current implementation
  uses `write.lock()` for the whole batch — correct. Ensure the `CfRouter` write
  also happens inside the same lock scope (or immediately before the seq is made
  visible to readers).
- On Windows (dev box), `sync_all()` may not guarantee durability; the FSV proof
  is only meaningful on aiwonder (Linux, ext4/ZFS with write barriers).
