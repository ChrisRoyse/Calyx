# PH06 · T02 — SST writer/reader byte-exact + big-endian key ordering

| Field | Value |
|---|---|
| **Phase** | PH06 — Memtable + LSM SSTable writer/reader |
| **Stage** | S1 — Aster storage core |
| **Crate** | `calyx-aster` |
| **Files** | `crates/calyx-aster/src/sst/mod.rs` (≤500) |
| **Depends on** | T01 (FrozenMemtable), PH05 (WAL ack) |
| **Axioms** | A15, A26 |
| **PRD** | `dbprdplans/04 §2/§8` |

## Goal

Harden the existing SST writer/reader with property and byte-level tests proving
that a round-trip flush/read is byte-exact, that big-endian multi-byte keys sort
lexicographically through the flush/range-scan cycle, that the file starts with
magic `CXS1`, and that a single-bit corruption in any record field returns
`CALYX_ASTER_CORRUPT_SHARD`. The existing tests cover basic scenarios; this card
adds the missing big-endian ordering test and proptest coverage.

## Build (checklist of concrete, code-level steps)

- [x] Add a proptest: for any sorted `Vec<(Vec<u8>, Vec<u8>)>` with distinct keys,
  `write_sst` + `SstReader::iter()` returns the exact same (key, value) pairs in
  the same order.
- [x] Add a test: write an SST with keys `[0x00_01_00_00u32, 0x00_02_00_00u32,
  0xFF_00_00_00u32]` encoded as 4-byte big-endian; assert `range` from
  `0x00_00_00_00` to `0xFF_FF_FF_FF` returns keys in ascending numeric order
  (big-endian byte order is lexicographically consistent with numeric order for
  unsigned integers).
- [x] Add a test: `SstReader::get` for a key that exists but is not in the bloom
  (impossible by construction) — confirm bloom is always loaded correctly by
  checking all written keys pass `bloom_may_contain`.
- [x] Add a test: empty SST (zero entries) writes and reads back with 0 entries
  and an empty range scan.
- [x] Verify `write_sst` uses `File::create(tmp)` + `write_all` + `sync_all` +
  `fs::rename(tmp, path)` (already implemented; confirm `sync_all` not
  `sync_data` is used so the directory entry is flushed).
- [x] Add a `SstReader::entry_count() -> usize` method that returns
  `self.index.len()` for FSV reporting.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: flush known 3-entry memtable; open SST; `iter()` returns exactly those
  3 entries byte-for-byte; file magic at offset 0 is `[0x43, 0x58, 0x53, 0x31]`.
- [x] proptest: `∀ sorted distinct-key entries`: round-trip through write/read is
  byte-exact; entry count preserved.
- [x] edge (≥3): (1) zero-entry SST → valid file, 0 range scan results; (2)
  single entry SST → point lookup finds it, range scan finds it; (3) big-endian
  u32 keys sort correctly through range scan.
- [x] fail-closed: flip byte in record body → `CALYX_ASTER_CORRUPT_SHARD` on
  `get`; flip header offset field → `CALYX_ASTER_CORRUPT_SHARD` on `open`.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** SST file at `/home/croyse/calyx/test-vault/cf/base/000001.sst`.
- **Readback:**
  ```
  xxd /home/croyse/calyx/test-vault/cf/base/000001.sst | head -2
  calyx readback --sst /home/croyse/calyx/test-vault/cf/base/000001.sst
  ```
- **Prove:** First 4 bytes are `43 58 53 31`; `calyx readback` output lists each
  key/value in the SST in ascending order, matching the known input exactly.
  The `entry_count()` reported by the reader equals the number of entries written.

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] FSV evidence (readback output / screenshot) attached to the PH06 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
