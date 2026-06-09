# PH05 · T04 — kill -9 crash drill + WAL FSV

| Field | Value |
|---|---|
| **Phase** | PH05 — WAL + group-commit + fsync |
| **Stage** | S1 — Aster storage core |
| **Crate** | `calyx-aster` |
| **Files** | `crates/calyx-aster/src/wal/tests.rs` (≤500), `crates/calyx-cli/src/main.rs` (CLI drill harness) |
| **Depends on** | T02 (replay correctness), T03 (group-commit batcher) |
| **Axioms** | A15, A16 |
| **PRD** | `dbprdplans/04 §7` |

## Goal

Prove — on aiwonder by reading raw bytes, not via a test harness return value —
that after a `kill -9` mid-write, WAL replay delivers the last acked record
byte-exact, the un-acked record is absent, and the torn tail is discarded with
`CALYX_ASTER_TORN_WAL`. This is the FSV gate for the entire PH05 phase.

## Build (checklist of concrete, code-level steps)

- [x] Add a `calyx wal-drill` CLI subcommand (in `calyx-cli`) that:
  1. Opens a `Wal` in a temp directory under `CALYX_HOME`.
  2. Appends N records (default 10) via `GroupCommitBatcher::submit`.
  3. Writes record N+1's bytes to the segment **without calling fsync**
     (simulate by writing directly to the underlying file after bypassing the
     batcher), then returns — leaving a partial record on disk.
  4. Prints `LAST_ACKED_SEQ=<n>` and `WAL_DIR=<path>` to stdout.
- [x] Add a `calyx wal-replay <dir>` CLI subcommand that calls `replay_dir` and
  prints each record's seq, payload hex, and `torn_tail` (if any) to stdout.
- [x] In tests: spawn `calyx wal-drill` as a subprocess; then spawn
  `calyx wal-replay <dir>`; parse stdout and assert last recovered seq ==
  `LAST_ACKED_SEQ` and no record with seq > `LAST_ACKED_SEQ` is present.
- [x] Add a `kill -9` variant: spawn the drill subprocess and send `SIGKILL`
  mid-append (after the group-commit window but before fsync returns) using
  `std::os::unix::process::CommandExt::kill`. Replay and assert invariants.
- [x] Verify `torn_tail.code == "CALYX_ASTER_TORN_WAL"` in the `wal-replay` output.
- [x] Document the exact `xxd` commands to read before/after in the phase GitHub
  issue (see FSV section below).

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit (subprocess): drill + replay round-trip: all N acked records present;
  no record at seq N+1; torn tail reported.
- [x] unit (subprocess): `kill -9` during append batch: replay returns exactly the
  records whose fsync completed before the kill; torn tail present.
- [x] proptest: for `n in 1..=8`: drill with n records, then manual truncate to
  mid-record, then replay — exactly `n-0` or `n-1` records depending on which
  was acked; never more than n.
- [x] edge (≥3): (1) empty WAL + truncate → 0 records, torn tail; (2) last record
  complete but no further write → n records, no torn tail; (3) two segments, torn
  in segment 1 → segment 0 fully replayed, segment 2 deleted.
- [x] fail-closed: corrupt magic in segment 0 → `replay_dir` returns
  `torn_tail.code == "CALYX_ASTER_TORN_WAL"` at offset 0, 0 records.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** WAL segment file at `/home/croyse/calyx/wal-drill/wal/00000000000000000000.wal`.
- **Readback (before kill):**
  ```
  xxd /home/croyse/calyx/wal-drill/wal/00000000000000000000.wal
  ```
  Expected: complete framed records for seqs 1..N; partial/zero bytes at end for
  the un-acked record.
- **Readback (after replay):**
  ```
  xxd /home/croyse/calyx/wal-drill/wal/00000000000000000000.wal
  calyx wal-replay /home/croyse/calyx/wal-drill/wal
  ```
  Expected: segment truncated to end of last complete record; `torn_tail:
  CALYX_ASTER_TORN_WAL at byte <offset>`; `last_seq: N`; no seq N+1 in output.
- **Prove:** The before→after delta is: file size decreases by the partial
  record bytes; the last complete record at offset `(N-1) * record_size` is
  intact; `xxd` shows `43 58 57 31` (magic) at offset 0 and at every record
  boundary. The un-acked record is absent — its bytes do not appear anywhere in
  the post-replay file.

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] FSV evidence (readback output / screenshot) attached to the PH05 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
