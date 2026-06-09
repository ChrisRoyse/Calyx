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
of dimension D in row-major SoA layout so SIMD loads read one dimension across
all vectors with a single pointer. This enables the SIMD-scan path in Sextant
(PH23). Writer produces a self-describing byte block that can be embedded as the
`value` in an SST record or written as a standalone file.

## Build (checklist of concrete, code-level steps)

- [x] Define `ArrowColumnChunk` format: `[magic: 4B "CXA1"] [version: u32 LE]
  [n_rows: u32 LE] [dim: u32 LE] [data: n_rows * dim * 4B f32 LE, row-major]`.
  Total header: 16 bytes.
- [x] Implement `fn encode_column_chunk(rows: &[[f32]]) -> Result<Vec<u8>>`:
  validates all rows have the same `dim`, writes the header + f32 data in
  row-major order (SoA semantics: row 0 first, then row 1, etc.).
- [x] Implement `fn decode_column_chunk(bytes: &[u8]) -> Result<ArrowChunkView>`:
  validates magic, version, checks byte length == 16 + n_rows*dim*4; returns a
  zero-copy view (slice reference into the input bytes).
- [x] `ArrowChunkView`: exposes `row(i: usize) -> &[f32]` (bounds-checked),
  `n_rows()`, `dim()`, `raw_bytes()`.
- [x] Ensure f32 data is 4-byte aligned; the header is 16 bytes (naturally
  aligned for f32).
- [x] Fail-closed: `decode_column_chunk` on wrong magic → `CALYX_ASTER_CORRUPT_SHARD`;
  wrong byte length → `CALYX_ASTER_CORRUPT_SHARD`.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: encode 3 vectors of dim 4 with known f32 values; decode; assert
  `row(0)` is byte-exact to the input (reinterpret as `[f32; 4]`); verify magic
  bytes at offset 0 are `[0x43, 0x58, 0x41, 0x31]` (`CXA1`).
- [x] proptest: for any `n in 1..=64, dim in 1..=128`: encode/decode round-trips
  with all values bit-identical.
- [x] edge (≥3): (1) n=1, dim=1 → 1-element chunk; (2) dim=0 → error; (3) rows
  with different dims → error; (4) empty byte slice → `CALYX_ASTER_CORRUPT_SHARD`.
- [x] fail-closed: bad magic → `CALYX_ASTER_CORRUPT_SHARD`; truncated data
  (1 byte short) → `CALYX_ASTER_CORRUPT_SHARD`.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** Arrow column chunk bytes embedded in an SST record value.
- **Readback:**
  ```
  xxd /home/croyse/calyx/test-vault/cf/slot_00/000001.sst | head -4
  calyx readback --cf slot_00 --sst /home/croyse/calyx/test-vault/cf/slot_00/000001.sst
  ```
- **Prove:** The value bytes for a slot vector key begin with `43 58 41 31`
  (`CXA1`), followed by `01 00 00 00` (version=1 LE), followed by n_rows and dim
  as u32 LE, followed by the raw f32 bytes. `calyx readback` prints the decoded
  vector values matching the original input.

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] FSV evidence (readback output / screenshot) attached to the PH06 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
