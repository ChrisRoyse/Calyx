# PH11 · T05 — Write-amp soak + cold-slot physical path FSV

| Field | Value |
|---|---|
| **Phase** | PH11 — Compaction + hot/cold tiering |
| **Stage** | S1 — Aster storage core |
| **Crate** | `calyx-aster` |
| **Files** | `crates/calyx-aster/src/compaction/tests.rs` (≤500), `crates/calyx-cli/src/main.rs` |
| **Depends on** | T03 (tiering), T04 (scheduler) |
| **Axioms** | A26 |
| **PRD** | `dbprdplans/04 §6`, `dbprdplans/24 §3` |

## Goal

The PH11 FSV gate: prove on aiwonder that (1) compaction runs safely with
concurrent reads (no partial reads during a compaction swap), (2) cold-tier slots
physically exist under the archive path, and (3) write-amp stays ≤ 2× (score_milli
≤ 2000) on a 1000-op soak. This is the final proof that the storage core is
production-ready for PH12+.

Post-sweep clarification #327: the shipped `calyx soak` is a storage stress and
readback tool over SST/WAL/tiering paths, while durable tier placement in normal
vault writes is covered by `VaultOptions::tiering_policy` and #295. The older
"ingest 1000 constellations" wording is the product-facing PH62 workflow shape,
not a missing PH11 storage-core requirement. Sweep residual #337 adds core
`CompactionReport` coverage that asserts the default `write_amp_milli <= 2000`
bound directly on a deterministic two-shard merge.

## Build (checklist of concrete, code-level steps)

- [x] Add `calyx soak --vault <path> --ops 1000 --threads 4` CLI subcommand:
  run 1000 deterministic storage operations across worker threads, trigger
  compaction at fixed intervals, and report `write_amp_milli` for each CF.
- [x] In core compaction coverage, assert that `CompactionReport::write_amp_milli
  <= 2000` for a deterministic per-CF merge. Product soak CLI enforcement stays
  a PH62 workflow concern unless reopened by a separate issue.
- [x] Add `calyx compact --vault <path> --cf <name>` CLI subcommand that runs one
  compaction for a specific CF and prints the `CompactionReport`.
- [x] Write a concurrent read/compact test: 2 reader threads each pinning a
  snapshot and reading 1000 keys; 1 compaction thread running `compact_cf` 10
  times concurrently; assert all reader reads return consistent values (no
  `CALYX_ASTER_CORRUPT_SHARD`, no wrong values).
- [x] Write a tiering test that uses `TieringPolicy::new` with temp-dir hot and
  archive roots; writes a `slot_00.raw` CF SST; asserts the file exists under the
  archive root, not the hot root.
- [x] Verify the soak does not leak file descriptors: count open fds before and
  after on aiwonder via `/proc/self/fd`.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit (concurrent): 2 readers + 1 compactor → no read errors over 10 compaction
  cycles.
- [x] unit: deterministic two-shard compaction has `write_amp_milli <= 2000`.
- [x] unit: `slot_00.raw` → archive root; `slot_00` (active) → hot root.
- [x] proptest: for `n in 1..=5 compaction rounds` with `m in 1..=100 entries`:
  all entries readable after each round; no data loss.
- [x] edge (≥3): (1) soak with 0 compactions (all below debt trigger) → write-amp
  metric still reported (1× = 1000 score_milli); (2) soak with forced compaction
  every op → write-amp bounded; (3) cold-tier path does not exist →
  `CALYX_DISK_PRESSURE`.
- [x] fail-closed: `write_tiered_sst` to a non-existent and uncreateable path →
  `CALYX_DISK_PRESSURE`.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** Archive-tier SST files at `/home/croyse/calyx/archive/cf/slot_00.raw/`;
  `CompactionReport::write_amp_milli` in soak output.
- **Readback:**
  ```
  calyx soak --vault /home/croyse/calyx/test-vault --ops 1000 --threads 4
  ls /home/croyse/calyx/archive/cf/slot_00.raw/
  calyx readback --cf base --vault /home/croyse/calyx/test-vault
  ```
- **Prove:** Soak output shows `write_amp_milli ≤ 2000` for each CF compaction
  report line. Archive path contains ≥1 `.sst` file for cold CFs (e.g.,
  `slot_00.raw`). `calyx readback --cf base` returns the persisted soak rows
  without error. No temp files (`*.sst.tmp`) remain in any CF dir.
  Screenshot of soak report and archive directory listing posted to PH11 GitHub
  issue. This is the Stage 1 exit proof.

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] FSV evidence (readback output / screenshot) attached to the PH11 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
