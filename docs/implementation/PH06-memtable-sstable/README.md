# PH06 — Memtable + LSM SSTable writer/reader

**Stage:** S1 — Aster storage core  ·  **Crate:** `calyx-aster`  ·
**PRD roadmap:** P0  ·  **Axioms:** A26

## Objective

Deliver a bounded in-RAM memtable that flushes to immutable, ordered SSTables;
a block-based mmap reader with a block index and bloom filter; range-scan
iteration that returns keys in big-endian order; and Arrow-compatible column
layout for slot columns. This is the LSM storage layer that all CFs write
through.

## Dependencies

- **Phases:** PH05 (WAL fsync contract — memtable flush is triggered after WAL
  ack), PH04 (CalyxError, bounded resource types)
- **Provides for:** PH07 (CF key routing writes memtable per-CF), PH09 (vault
  write path flushes memtable to SST), PH10 (manifest captures SST refs),
  PH11 (compaction merges SSTs)

## Current state (build off what exists)

`memtable.rs` and `sst/mod.rs` (+ `sst/bloom.rs`) are already written and
compile with tests. The memtable has bounded byte-cap enforcement with
`CALYX_BACKPRESSURE` on overflow. The SST writer writes sorted key/value records
with a block index and bloom filter; the reader uses mmap, binary search index,
bloom probe for point lookups, and linear scan for range reads.

**What remains:**

- The SST format uses **little-endian** CRC and header offsets; the PRD requires
  **big-endian-ordered keys** for range scans (`04 §4`). The key encoding is the
  caller's responsibility (CF key layer), but the SST iterator must preserve the
  exact byte order. This is already correct (BTreeMap is LE-byte-comparable only
  by the caller's key bytes) — add a test that verifies big-endian multi-byte
  keys sort lexicographically as expected through a flush/read cycle.
- No Arrow-layout column-block writer yet for slot columns. Need a thin
  `ArrowColumnChunk` writer/reader that packs `f32` vectors in SoA order (SIMD-
  friendly) alongside the existing record format for non-vector CFs.
- The memtable has no `freeze`/`rotate` API — flush and create a new empty one.
  Add `Memtable::freeze() -> FrozenMemtable` to make the flush/rotate explicit.
- No multi-SST read layer (needed for PH07 CF dispatch) — add a simple
  `SstLevel` that queries a list of SSTs newest-first.

## Deliverables (file plan, each ≤500 lines)

| File | Responsibility |
|---|---|
| `src/memtable.rs` | Bounded memtable, `freeze()` / rotate API |
| `src/sst/mod.rs` | SST writer (atomic rename), mmap reader, bloom, range scan |
| `src/sst/bloom.rs` | Bloom filter (already present; harden with proptest) |
| `src/sst/arrow.rs` | Arrow-layout `f32` column chunk writer/reader (SoA, ≤500 L) |
| `src/sst/level.rs` | Multi-SST level: newest-first point lookup + ordered range merge |

## Tasks (atomic — all must pass for the phase to be DONE)

| Card | Title | Depends |
|---|---|---|
| T01 | Memtable freeze/rotate + backpressure proptest | — |
| T02 | SST writer/reader byte-exact + big-endian key ordering | T01 |
| T03 | Bloom filter proptest (no false negatives) | T02 |
| T04 | Arrow-layout f32 column chunk writer/reader | T02 |
| T05 | Multi-SST level: newest-first point lookup + range merge | T02, T03 |

## FSV exit gate (the phase is DONE only when this is byte-proven on aiwonder)

Flush a known memtable to an SST on aiwonder; read back every key byte-exact:

```
calyx readback --cf base --sst /home/croyse/calyx/test-vault/cf/base/000001.sst
xxd /home/croyse/calyx/test-vault/cf/base/000001.sst | head -2
```

Expected: magic `43 58 53 31` (`CXS1`) at offset 0; range scan returns keys in
ascending byte order; bloom never false-negatives on any key present in the file.
Evidence posted to PH06 GitHub issue.

## Risks / landmines

- Arrow SoA layout for slot columns must be SIMD-aligned (16-byte row alignment);
  use `repr(C)` or explicit padding — misalignment causes silent performance loss.
- `fs::rename` across ZFS datasets fails with `EXDEV`; SST temp files must be
  created in the destination CF directory (same dataset), not in `/tmp`.
- Bloom filter false-positive rate: use 10 bits/key and 7 hash functions
  (standard double-hashing with two seeded xxh3 passes) to keep FPR < 1%.
