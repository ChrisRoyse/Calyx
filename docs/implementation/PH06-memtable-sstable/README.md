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

## Status — DONE ✅ (Stage 1; FSV-signed-off 2026-06-07, commit 8dcddaa)

Shipped in `calyx-aster`:
- `memtable.rs` — bounded memtable, `CALYX_BACKPRESSURE` at cap, `freeze()`→`FrozenMemtable`, `needs_flush()` at 90%.
- `sst/mod.rs` — `write_sst` (atomic `.sst.tmp`+rename), `SstReader` mmap + binary-search index + per-record crc, magic `CXS1`; corrupt crc / invalid offsets fail closed.
- `sst/bloom.rs` — blake3 double-hash bloom; no false negatives; seeded FPR <1%.
- `sst/arrow.rs` — Arrow SoA f32 column chunk (`CXA1`), bit-exact roundtrip.
- `sst/level.rs` — multi-SST level, newest-wins point lookup + ordered range merge.

FSV evidence: GitHub issue #23 (`[CONTEXT] You are here`); Stage-1 evidence root `/home/croyse/calyx/data/fsv-stage1-exit-20260607105216`.

### Post-sweep slot-column materialization
- The live slot CF write path intentionally stores slot values via
  `vault/encode.rs::encode_slot_vector` (a byte-exact dense/sparse/multi codec);
  those row bytes remain Aster's CRUD/recovery source of truth.
- #341 plus post-sweep SoA hardening adds a derived dense sidecar materializer
  in `vault/slot_column.rs`: it scans visible row-encoded `slot_NN` CF values at
  a pinned snapshot, validates dense equal-dim rows, writes dimension-contiguous
  column-major f32 payload bytes to `slot-column.cxa1` (`CXA1`) plus
  `slot-column-manifest.json` (`CXSC1`), and reads the artifact back with hash
  verification. Evidence root:
  `/home/croyse/calyx/data/fsv-issue341-slot-column-soa-20260609-b960c58`.

## Deliverables (file plan, each ≤500 lines)

| File | Responsibility |
|---|---|
| `src/memtable.rs` | Bounded memtable, `freeze()` / rotate API |
| `src/sst/mod.rs` | SST writer (atomic rename), mmap reader, bloom, range scan |
| `src/sst/bloom.rs` | Bloom filter (already present; harden with proptest) |
| `src/sst/arrow.rs` | Arrow-layout `f32` column chunk writer/reader (SoA, ≤500 L) |
| `src/sst/level.rs` | Multi-SST level: newest-first point lookup + ordered range merge |
| `src/vault/slot_column.rs` | Derived dense slot-column materialization sidecar + readback |

## Tasks (atomic — all must pass for the phase to be DONE)

| Card | Title | Depends |
|---|---|---|
| T01 | Memtable freeze/rotate + backpressure proptest | — |
| T02 | SST writer/reader byte-exact + big-endian key ordering | T01 |
| T03 | Bloom filter proptest (no false negatives) | T02 |
| T04 | Arrow-layout f32 column chunk writer/reader | T02 |
| T05 | Multi-SST level: newest-first point lookup + range merge | T02, T03 |

## FSV exit gate (the phase is DONE only when this is byte-proven on aiwonder)

> ✅ **Achieved** — byte-proven on aiwonder; evidence in GitHub issue #23 (Stage-1 FSV root `/home/croyse/calyx/data/fsv-stage1-exit-20260607105216`).

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
