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

## Current state (build off what exists)

`mvcc/mod.rs`, `mvcc/store.rs`, `mvcc/lease.rs` are already written with:
- `SeqAllocator`: monotonic `AtomicU64`, `allocate()` returns next seq.
- `VersionedCfStore`: in-memory `RwLock<HashMap<(CF, key), Vec<VersionedValue>>>`.
  `commit_batch` allocates one seq for the whole group. `read_at`/`read_batch`
  filter by `seq <= snapshot.seq`. `pin_snapshot` with clock injection.
- `Freshness::FreshDerived` / `StaleOk { max_lag }`.
- `ReaderLease` with `max_age_ms` expiry; `ensure_live` checks lease age.
- `Snapshot` combining seq + freshness + lease.
- `mvcc/tests.rs` exists.

**What remains:**
- `VersionedCfStore` is purely in-memory. It needs a bridge to `CfRouter`: on
  `commit_batch`, in addition to writing the in-memory version chain, also write
  to the CF router (so writes land on disk). This is the main integration task.
- Snapshot `pin_snapshot` in `AsterVault` (vault.rs) needs to use a proper
  `SeqAllocator` whose value is recoverable from WAL replay after a crash (PH10
  sets the start seq after recovery).
- Concurrency test: concurrent writer + reader must never expose a partial
  constellation (i.e., base CF row visible but slot CF rows not yet visible).
- `read_batch` must guarantee atomicity at the pinned seq across all CF rows.

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
