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

## Status — DONE ✅ (Stage 1; FSV-signed-off 2026-06-07, commit 8dcddaa)

Shipped in `calyx-aster`:
- `wal/record.rs` — framing `CXW1` + seq(LE) + len(LE) + crc32 + payload; `encode`/`decode_at`; torn detection; proptest roundtrip + golden edge cases.
- `wal/mod.rs` — `append_batch` (single `sync_data` per batch), `replay_dir` truncates torn tail + removes later segments, segment rotation on byte cap with `sync_all` on rotate; `CALYX_ASTER_TORN_WAL` on torn tail.
- `wal/batch.rs` — `GroupCommitBatcher`, ≤2 ms window driven by injected `Clock`; `validate_window` fails closed on >2 ms.
- `wal/segment.rs`, `wal/tests.rs`. CLI: `wal-drill`, `wal-replay`, `wal-batch-demo`, `readback --wal`.

FSV evidence: GitHub issue #23 (`[CONTEXT] You are here`); Stage-1 evidence root `/home/croyse/calyx/data/fsv-stage1-exit-20260607105216`.

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

> ✅ **Achieved** — byte-proven on aiwonder; evidence in GitHub issue #23 (Stage-1 FSV root `/home/croyse/calyx/data/fsv-stage1-exit-20260607105216`).

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
  rotation) must be flushed with `sync_all` or a parent-dir fsync. Current code
  uses `sync_data()` for ordinary batch appends and `sync_all()` on segment
  rotation, matching the PH05 durability boundary.
- `EXDEV` if staging WAL temp files cross ZFS dataset boundaries — stage temp
  files inside the WAL directory (same dataset) to avoid this.
- Group-commit timer: use the `Clock` trait for the deadline, not
  `std::time::Instant::now()`, so tests can inject a `FixedClock`.
