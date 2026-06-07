# PH05 — WAL + group-commit + fsync

**Stage:** S1 — Aster storage core  ·  **Crate:** `calyx-aster`  ·
**PRD roadmap:** P0  ·  **Axioms:** A15, A16

## Objective

Deliver a durable write-ahead log with a group-commit window of ≤2 ms, per-record
CRC framing, segment rotation, fsync-before-ack, and torn-tail discard on replay.
The WAL is the source of truth for all un-compacted writes; no constellation is
considered durable until its WAL record is fsync'd and acked. `CALYX_ASTER_TORN_WAL`
is surfaced whenever replay encounters a torn tail.

## Dependencies

- **Phases:** PH04 (calyx-core structs, `Clock` trait, `CalyxError` catalog)
- **Provides for:** PH06 (memtable flush trigger), PH09 (write path integration),
  PH10 (manifest recovery ordering)

## Current state (build off what exists)

`wal/mod.rs`, `wal/record.rs`, and `wal/segment.rs` are already written and
compile. The WAL has:
- Record framing: magic `CXW1` + seq (u64 LE) + len (u32 LE) + crc32 + payload.
- `append_batch` with a single `sync_data()` after writing all records in the
  batch (group-commit in intent; the batcher loop is not yet externally driven by
  a time window).
- `replay_dir` that truncates torn tails and removes later segments.
- Segment rotation on byte cap (`max_segment_bytes`, default 64 MiB).
- `DEFAULT_GROUP_COMMIT_WINDOW: Duration = Duration::from_millis(2)` is declared
  but the caller (vault) drives batching; no dedicated timer thread yet.

**What remains:**
- A `GroupCommitBatcher` that collects callers for ≤2 ms, then flushes as one
  `append_batch` call. The current vault uses `commit_batch` against the in-memory
  `VersionedCfStore`; the WAL is not yet wired into the vault write path.
- A WAL integration test that proves `kill -9` durability by writing, crashing
  the process (or simulating by truncating the file mid-byte), replaying, and
  asserting bytes.
- A property test that `decode(encode(seq, payload)) == (seq, payload)` for all
  valid inputs.
- FSV drill documented in the phase GitHub issue.

## Deliverables (file plan, each ≤500 lines)

| File | Responsibility |
|---|---|
| `src/wal/mod.rs` | `Wal`, `WalOptions`, `append_batch`, `replay_dir`, `TornTail` — harden group-commit batcher |
| `src/wal/record.rs` | `encode`/`decode_at`, CRC framing — already complete; proptest coverage |
| `src/wal/segment.rs` | Segment naming helpers — already complete |
| `src/wal/batch.rs` | `GroupCommitBatcher`: timed coalescing loop, ≤2 ms window, flush trigger |
| `src/wal/tests.rs` | Integration tests: torn-tail recovery, segment rotation, group-commit timing |

## Tasks (atomic — all must pass for the phase to be DONE)

| Card | Title | Depends |
|---|---|---|
| T01 | Record encode/decode + proptest | — |
| T02 | Segment rotation + replay correctness | T01 |
| T03 | Group-commit batcher (≤2 ms window) | T01 |
| T04 | kill-9 crash drill + WAL FSV | T02, T03 |

## FSV exit gate (the phase is DONE only when this is byte-proven on aiwonder)

Run the vault write loop on aiwonder; issue `kill -9` mid-write batch; restart;
replay the WAL directory. Proof:

```
xxd /home/croyse/calyx/test-vault/wal/00000000000000000000.wal | head -4
```

Expected: last acked record's bytes present at the correct offset; partially
written record's bytes absent (segment truncated to last good record boundary).
`CALYX_ASTER_TORN_WAL` code returned if a torn tail was found. Evidence
(terminal screenshot + xxd output) posted to the PH05 GitHub issue.

## Risks / landmines

- `sync_data()` vs `sync_all()`: on Linux metadata updates (file length after
  rotation) must be flushed with `sync_all` or a parent-dir fsync; use
  `sync_all()` on segment open/rotate. Current code uses `sync_data()` for
  batch appends (correct) and `sync_data()` before rotation (should be
  `sync_all()`).
- `EXDEV` if staging WAL temp files cross ZFS dataset boundaries — stage temp
  files inside the WAL directory (same dataset) to avoid this.
- Group-commit timer: use the `Clock` trait for the deadline, not
  `std::time::Instant::now()`, so tests can inject a `FixedClock`.
