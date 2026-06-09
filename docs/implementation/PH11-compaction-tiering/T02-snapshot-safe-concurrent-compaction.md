# PH11 · T02 — Snapshot-safe concurrent compaction (reads during merge)

| Field | Value |
|---|---|
| **Phase** | PH11 — Compaction + hot/cold tiering |
| **Stage** | S1 — Aster storage core |
| **Crate** | `calyx-aster` |
| **Files** | `crates/calyx-aster/src/compaction/mod.rs` (≤500), `crates/calyx-aster/src/compaction/tests.rs` (≤500) |
| **Depends on** | T01 (debt meter), PH06 T05 (SstLevel) |
| **Axioms** | A26 |
| **PRD** | `dbprdplans/04 §6` |

## Goal

Prove that `CompactionCatalog::compact_cf` is safe to call concurrently with
readers: a reader that pins a `CompactionSnapshot` before a compaction swap begins
can still read all its data to completion after the swap, because the old
`Arc<Vec<SstShard>>` is held alive by the snapshot. Also prove the swap is atomic:
after `compact_cf` returns, new `pin_snapshot` calls see only the compacted shard.

## Build (checklist of concrete, code-level steps)

- [x] Write concurrent test: 4 SST shards for CF `Base`; a reader thread pins a
  snapshot; a writer thread calls `catalog.compact_cf(Base, output_path,
  unlimited)` concurrently; after the writer returns, the reader still reads all
  entries from its pinned snapshot without error.
- [x] Write test: after `compact_cf` completes, `catalog.pin_snapshot().shard_count()`
  for CF `Base` is 1 (not 4); all entries from the 4 input shards are in the
  output; the output SST is sorted.
- [x] Write test: `compact_cf` with a corrupt input SST → `CALYX_ASTER_CORRUPT_SHARD`
  from `SstReader::iter()`; the catalog is NOT modified (old shards remain).
- [x] Ensure `compact_cf` does not hold the catalog `write` lock during the
  multi-SST merge (which may take seconds for large CFs); it only holds the lock
  for the pointer swap at the end.
- [x] Add `CompactionCatalog::shard_count_for_cf(&self, cf: ColumnFamily) -> usize`
  helper for tests.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] concurrent: pin snapshot → compact → reader gets all entries (no partial read,
  no error).
- [x] unit: 4 shards → 1 after compact; all entries merged; sorted.
- [x] unit: corrupt shard → compact returns Err; catalog unchanged.
- [x] proptest: for `n in 1..=8 shards`, each with `m in 1..=10 entries`:
  compact → output has all unique keys (newest version for duplicates).
- [x] edge (≥3): (1) single shard → trivially compacted to copy (same entries);
  (2) two shards with same keys → newest shard's value wins; (3) all shards for
  the CF but none for another CF → other CF unchanged.
- [x] fail-closed: corrupt input SST during compact → `CALYX_ASTER_CORRUPT_SHARD`;
  original shards unmodified.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** CF directory before and after compaction at `/home/croyse/calyx/test-vault/cf/base/`.
- **Readback:**
  ```
  ls /home/croyse/calyx/test-vault/cf/base/
  calyx compact --vault /home/croyse/calyx/test-vault --cf base
  ls /home/croyse/calyx/test-vault/cf/base/
  calyx readback --cf base --vault /home/croyse/calyx/test-vault
  ```
- **Prove:** Before compact: N SST files in the base CF dir. After compact: 1 SST
  file (or N-1 → N files minus inputs plus 1 output). `calyx readback` returns
  all constellations byte-exact. Screenshot posted to PH11 GitHub issue.

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] FSV evidence (readback output / screenshot) attached to the PH11 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
