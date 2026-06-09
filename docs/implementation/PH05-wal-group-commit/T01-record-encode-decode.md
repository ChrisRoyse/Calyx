# PH05 · T01 — WAL record encode/decode + proptest

| Field | Value |
|---|---|
| **Phase** | PH05 — WAL + group-commit + fsync |
| **Stage** | S1 — Aster storage core |
| **Crate** | `calyx-aster` |
| **Files** | `crates/calyx-aster/src/wal/record.rs` (≤500), `crates/calyx-aster/src/wal/tests.rs` (≤500) |
| **Depends on** | PH04 (CalyxError catalog) |
| **Axioms** | A16 |
| **PRD** | `dbprdplans/04 §5/§7` |

## Goal

Harden the WAL record framing layer with a complete proptest suite that proves
`decode_at(encode(seq, payload)) == (seq, payload)` for arbitrary valid inputs,
and that every corrupt-byte case returns a `Torn` status rather than silently
succeeding or panicking. The record layer is already written; this card adds
property/fuzz coverage and locks in the byte layout as a regression baseline.

## Build (checklist of concrete, code-level steps)

- [x] Add `proptest` dev-dependency to `calyx-aster/Cargo.toml` if not present.
- [x] Write `proptest!` macro test: for any `(seq: u64, payload: Vec<u8>)` with
  `payload.len() <= 64 * 1024 * 1024`, assert
  `decode_at(encode(seq, payload)) == Complete(DecodedRecord { seq, payload, .. })`.
- [x] Write deterministic golden test: `encode(42, b"hello")` produces the exact
  17-byte sequence `[CXW1 magic (LE) | seq 42 (8 B LE) | len 5 (4 B LE) |
  crc32 (4 B LE) | b"hello"]`; compute expected CRC in test, assert byte-exact.
- [x] Edge case: zero-length payload encodes and round-trips.
- [x] Edge case: payload length = `MAX_RECORD_BYTES` (64 MiB) encodes without
  truncation; payload length = `MAX_RECORD_BYTES + 1` returns
  `io::Error(InvalidInput)`.
- [x] Fail-closed: flip one bit in the CRC field of an encoded record; assert
  `decode_at` returns `Torn { offset: 0, message: contains("crc mismatch") }`.
- [x] Fail-closed: truncate encoded bytes to `HEADER_LEN - 1`; assert
  `Torn { message: contains("partial WAL header") }`.
- [x] Fail-closed: truncate encoded bytes to exactly the header with zero payload
  bytes when `len > 0`; assert `Torn { message: contains("partial WAL payload") }`.
- [x] Fail-closed: write magic bytes `0x00000000`; assert
  `Torn { message: contains("bad WAL magic") }`.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: `encode(42, b"hello")` → byte-exact golden (compute CRC in test with
  `crc32fast::Hasher`, assert `output[0..4] == *b"CXW1"` and `output[4..12] ==
  42u64.to_le_bytes()`).
- [x] proptest: `decode_at(encode(seq, payload)) == Complete(DecodedRecord { seq,
  payload })` for all `(seq: u64, payload in 0..=1024 bytes)`.
- [x] edge (≥3): (1) zero-byte payload round-trips; (2) max payload length
  accepted; (3) max+1 payload rejected with `InvalidInput`; (4) partial header
  → `Torn`.
- [x] fail-closed: single-bit CRC flip → `Torn` with message containing
  `"crc mismatch"` and `code == "CALYX_ASTER_TORN_WAL"` via `TornTail::error()`.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** WAL segment file `wal/00000000000000000000.wal`.
- **Readback:** `xxd /home/croyse/calyx/test-vault/wal/00000000000000000000.wal | head -4`
- **Prove:** bytes 0–3 are `43 58 57 31` (`CXW1` LE); bytes 4–11 are seq as 8 B
  little-endian; bytes 16–19 are the CRC. Running the unit test binary with
  `-- --nocapture` on aiwonder and dumping the encoded bytes proves the layout
  without a real file — the real on-disk proof comes in T04.

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] FSV evidence (readback output / screenshot) attached to the PH05 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
