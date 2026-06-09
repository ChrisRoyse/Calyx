# PH06 · T04 — Arrow-layout f32 column chunk writer/reader

| Field | Value |
|---|---|
| **Phase** | PH06 — Memtable + LSM SSTable writer/reader |
| **Stage** | S1 — Aster storage core |
| **Crate** | `calyx-aster` |
| **Files** | `crates/calyx-aster/src/sst/arrow.rs` (≤500) |
| **Depends on** | T02 (SST writer) |
| **Axioms** | A26 |
| **PRD** | `dbprdplans/04 §2`, `dbprdplans/23 §2` (SoA columns) |

## Goal

Implement a compact Arrow-compatible SoA (Structure of Arrays) column chunk
writer and mmap reader for slot vectors. Each chunk stores N dense f32 vectors
of dimension D in dimension-contiguous column-major layout: column 0 for all
rows, then column 1 for all rows, and so on. This lets SIMD scans read one
dimension across all vectors with a single contiguous range. Writer produces a
self-describing byte block that can be embedded as the `value` in an SST record
or written as a standalone file.

## Build (checklist of concrete, code-level steps)

- [x] Define `ArrowColumnChunk` format: `[magic: 4B "CXA1"] [version: u32 LE]
  [n_rows: u32 LE] [dim: u32 LE] [data: n_rows * dim * 4B f32 LE,
  column-major by dimension]`. Total header: 16 bytes.
- [x] Implement `fn encode_column_chunk(rows: &[[f32]]) -> Result<Vec<u8>>`:
  validates all rows have the same `dim`, writes the header + f32 data in
  dimension-contiguous order (`col_0[row_0..row_n]`, then `col_1[...]`).
- [x] Implement `fn decode_column_chunk(bytes: &[u8]) -> Result<ArrowChunkView>`:
  validates magic, version, checks byte length == 16 + n_rows*dim*4; returns a
  readback view that preserves the raw bytes and exposes rows through a
  transposed owned row buffer.
- [x] `ArrowChunkView`: exposes `row(i: usize) -> &[f32]` (bounds-checked),
  `n_rows()`, `dim()`, `raw_bytes()`.
- [x] Ensure f32 data is 4-byte aligned; the header is 16 bytes (naturally
  aligned for f32).
- [x] Fail-closed: `decode_column_chunk` on wrong magic → `CALYX_ASTER_CORRUPT_SHARD`;
  wrong byte length → `CALYX_ASTER_CORRUPT_SHARD`.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: encode 3 vectors of dim 4 with known f32 values; decode; assert
  `row(0)` is byte-exact to the input (reinterpret as `[f32; 4]`); verify magic
  bytes at offset 0 are `[0x43, 0x58, 0x41, 0x31]` (`CXA1`) and payload order is
  dimension-contiguous.
- [x] proptest: for any `n in 1..=64, dim in 1..=128`: encode/decode round-trips
  with all values bit-identical.
- [x] edge (≥3): (1) n=1, dim=1 → 1-element chunk; (2) dim=0 → error; (3) rows
  with different dims → error; (4) empty byte slice → `CALYX_ASTER_CORRUPT_SHARD`.
- [x] fail-closed: bad magic → `CALYX_ASTER_CORRUPT_SHARD`; truncated data
  (1 byte short) → `CALYX_ASTER_CORRUPT_SHARD`.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** live slot CF row bytes remain row-encoded Aster CRUD/recovery bytes;
  the derived materialized sidecar is `slot-column.cxa1` plus
  `slot-column-manifest.json`.
- **Readback:**
  ```
  xxd /home/croyse/calyx/data/fsv-issue341-slot-column-soa-20260609-b960c58/vault/cf/slot_06/*.sst | head -6
  xxd /home/croyse/calyx/data/fsv-issue341-slot-column-soa-20260609-b960c58/materialized/slot_06/slot-column.cxa1 | head -4
  cat /home/croyse/calyx/data/fsv-issue341-slot-column-soa-20260609-b960c58/slot-column-readback.json
  ```
- **Prove:** the live `slot_06` SST value contains dense row-codec tag `00` and
  not `CXA1`; the derived materialized chunk begins `43 58 41 31` (`CXA1`) with
  version `01 00 00 00`, `rows=3`, `dim=4`; the payload is
  dimension-contiguous by the manifest `CxId` order; the manifest is `CXSC1`,
  lists the exact `CxId` order, and its chunk SHA-256 matches the bytes. Edges
  read back empty slot and non-dense slot as `CALYX_STALE_DERIVED`, and
  corrupted chunk bytes plus a path-traversal manifest as
  `CALYX_ASTER_CORRUPT_SHARD`.

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] FSV evidence (readback output / screenshot) attached to the PH06 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
