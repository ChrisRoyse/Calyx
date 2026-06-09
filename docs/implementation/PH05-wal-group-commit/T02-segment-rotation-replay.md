# PH05 · T02 — Segment rotation + replay correctness

| Field | Value |
|---|---|
| **Phase** | PH05 — WAL + group-commit + fsync |
| **Stage** | S1 — Aster storage core |
| **Crate** | `calyx-aster` |
| **Files** | `crates/calyx-aster/src/wal/mod.rs` (≤500), `crates/calyx-aster/src/wal/segment.rs` (≤500), `crates/calyx-aster/src/wal/tests.rs` (≤500) |
| **Depends on** | T01 (record framing verified) |
| **Axioms** | A15, A16 |
| **PRD** | `dbprdplans/04 §5/§7` |

## Goal

Prove that segment rotation happens correctly (new file opened when
`max_segment_bytes` is exceeded), that `replay_dir` replays all records across
multiple segments, stops at the first torn record and truncates it, removes later
segments, and returns `CALYX_ASTER_TORN_WAL` for the torn tail. All tests run
against real files in `tempdir`.

## Build (checklist of concrete, code-level steps)

- [x] Fix `rotate_if_needed`: replace `sync_data()` before rotation with
  `sync_all()` so file length metadata is flushed to disk before the new segment
  is opened.
- [x] Write integration test: open a `Wal` with `max_segment_bytes = 64`, append
  three records that sum to > 64 bytes; assert that two segment files exist in
  the WAL directory (`00…0.wal`, `00…1.wal`).
- [x] Write integration test: `Wal::open` on an existing directory with two
  segments resumes with `next_seq` equal to `last_replayed_seq + 1`.
- [x] Write integration test: `replay_dir` across two segments returns records in
  ascending seq order.
- [x] Write integration test: inject a torn tail by truncating the last segment to
  mid-record (remove last 4 bytes of the final encoded record); call `replay_dir`;
  assert `torn_tail.is_some()`, that the torn-tail `code == "CALYX_ASTER_TORN_WAL"`,
  and that the last replayed record seq is the one *before* the torn record.
- [x] Write integration test: two segments exist; torn record is in segment 0;
  assert segment 1 is deleted by `replay_dir` and only segment 0 (truncated) remains.
- [x] Ensure `open_append_file` uses `sync_all()` after creating a new segment
  (so the directory entry is flushed).

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: write 3 records fitting in one 4096-byte cap segment → 1 segment file.
- [x] unit: write 3 records that overflow a 64-byte cap → exactly 2 segment files;
  replay returns all 3 records in seq order.
- [x] proptest: `∀ n in 1..=20 records`: after `append` × n and `replay_dir`,
  recovered seqs == `[1..=n]`.
- [x] edge (≥3): (1) WAL dir does not exist on `open` → created; (2) empty WAL
  dir replays to 0 records, no torn tail; (3) torn record mid-segment 0 of 2 →
  segment 1 removed, segment 0 truncated.
- [x] fail-closed: torn tail `code` field == `"CALYX_ASTER_TORN_WAL"`;
  `TornTail::error().code == "CALYX_ASTER_TORN_WAL"`.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** Segment files under `/home/croyse/calyx/test-vault/wal/`.
- **Readback:**
  ```
  ls -la /home/croyse/calyx/test-vault/wal/
  xxd /home/croyse/calyx/test-vault/wal/00000000000000000000.wal
  xxd /home/croyse/calyx/test-vault/wal/00000000000000000001.wal
  ```
- **Prove:** Each segment file ends cleanly at a record boundary (no partial
  header bytes). The last byte of segment N equals the last byte of the last
  complete record appended to that segment. Torn-tail scenario: segment file size
  equals the offset of the torn record's start, and the later segment file does
  not exist.

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] FSV evidence (readback output / screenshot) attached to the PH05 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
