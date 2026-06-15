# PH35 · T02 — Binary codec (encode/decode) round-trip

| Field | Value |
|---|---|
| **Phase** | PH35 — Hash-chain append-only CF (in group-commit) |
| **Stage** | S7 — Ledger Provenance |
| **Crate** | `calyx-ledger` |
| **Files** | `crates/calyx-ledger/src/entry.rs` (≤500) |
| **Depends on** | T01 (this phase) |
| **Axioms** | A15, A16 |
| **PRD** | `dbprdplans/11 §2`, `04 §5` |

## Goal

Implement a deterministic, length-delimited binary codec for `LedgerEntry` so
entries can be written to the `ledger` CF and the WAL as raw bytes and decoded
back byte-exact. The codec must be stable across restarts: the same entry always
encodes to the same bytes, which is required for `entry_hash` reproducibility
and FSV readback with `xxd`.

**Current implementation (#243, commit `9ff808e`):** codec logic is split into
`crates/calyx-ledger/src/codec.rs` to keep `.rs` files below 500 lines. The
crate re-exports `encode`, `decode`, and `decode_header`. The golden codec hex
starts with `000000000000002a` (seq=42), has `prev_hash` at offsets 8-39 and
kind wire code `01` at offset 40, and is read back with `xxd` in the FSV root.

## Build (checklist of concrete, code-level steps)

- [x] `fn encode(entry: &LedgerEntry) -> Vec<u8>` — fixed-layout:
  `[seq(8)] [prev_hash(32)] [kind(1)] [subject_tag(1)] [subject_bytes(var, length-prefixed u16 BE)]
   [payload_len(4 BE)] [payload_bytes] [actor_tag(1)] [actor_bytes(var, length-prefixed u16 BE)]
   [ts(8)] [entry_hash(32)]`
  — no padding, no alignment gaps; total length deterministic given inputs.
- [x] `fn decode(bytes: &[u8]) -> Result<LedgerEntry>` — parse the fixed layout
  above; return `CalyxError::LedgerCorrupt` (new structured error code
  `CALYX_LEDGER_CORRUPT`) if any length field extends past the buffer.
- [x] `fn decode_header(bytes: &[u8]) -> Result<(u64, [u8;32])>` — fast-path
  decode of only `seq` + `prev_hash` for chain-link verification without full
  decode (used by `verify_chain` in PH36).
- [x] After decode, re-verify `entry_hash` via `LedgerEntry::verify()`; if it
  fails return `CALYX_LEDGER_CORRUPT` with `seq` in the structured payload.
- [x] Add `CALYX_LEDGER_CORRUPT` to the `calyx-core` error catalog
  (`crates/calyx-core/src/error.rs`) with remediation string
  `"ledger CF integrity violation — run verify_chain to identify range"`.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: `decode(encode(entry)) == entry` for a fixed known entry (seq=42,
  kind=Measure, payload=b"synthetic"); assert byte-exact.
- [x] unit: encode a known entry and assert the output bytes match a hard-coded
  golden byte vector (regression test for codec stability).
- [x] proptest: `decode(encode(x)) == x` for arbitrary valid `LedgerEntry`
  values (round-trip invariant).
- [x] edge (≥3): zero-length payload; max-length `subject_bytes` (255 bytes);
  single-byte actor id; `seq=0` genesis entry.
- [x] fail-closed: truncated buffer (1 byte short of `payload_len`) →
  `CALYX_LEDGER_CORRUPT`; entry with flipped `entry_hash` byte → `CALYX_LEDGER_CORRUPT`
  after decode re-verify; empty slice → `CALYX_LEDGER_CORRUPT`.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `crates/calyx-ledger/src/entry.rs` codec output bytes
- **Readback:** `cargo test -p calyx-ledger -- --nocapture codec_golden 2>&1`
  prints the encoded bytes of the golden entry; pipe through `xxd` and confirm
  offsets 0–7 = seq BE, offsets 8–39 = prev_hash, offset 40 = kind wire code.
- **Prove:** before: no codec exists; after: golden test passes and prints the
  same 32-byte `entry_hash` as T01; `decode(encode(x)) == x` proptest passes;
  truncated input returns `CALYX_LEDGER_CORRUPT` (not a panic).
- **Post-implementation readback:** #243 FSV root
  `/home/croyse/calyx/data/fsv-issue243-ledger-codec-20260608`; `codec_golden.xxd`
  contains the golden codec bytes, `codec_fail_closed.txt` proves corrupt input
  fails closed, and `codec_roundtrip.txt` proves the round-trip proptest.

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] FSV evidence attached to #243
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
