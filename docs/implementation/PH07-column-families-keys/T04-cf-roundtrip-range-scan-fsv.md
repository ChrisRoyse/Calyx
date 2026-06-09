# PH07 · T04 — CF round-trip + range-scan FSV

| Field | Value |
|---|---|
| **Phase** | PH07 — Column families + key encoding |
| **Stage** | S1 — Aster storage core |
| **Crate** | `calyx-aster` |
| **Files** | `crates/calyx-aster/src/cf/tests.rs` (≤500), `crates/calyx-cli/src/main.rs` |
| **Depends on** | T02 (collision check), T03 (CF router) |
| **Axioms** | A15, A16 |
| **PRD** | `dbprdplans/04 §4` |

## Goal

Prove on aiwonder — by reading each CF's actual SST bytes — that one row written
to each of the seven association-native CFs (`base`, `slot_00`, `xterm`,
`scalars`, `anchors`, `ledger`, `online`) round-trips byte-exact, and that a
range scan on `base` and `ledger` returns keys in the expected big-endian
ascending order. `CALYX_ASTER_CORRUPT_SHARD` is returned on hash mismatch.

## Build (checklist of concrete, code-level steps)

- [x] Add `calyx readback` CLI subcommand with flags `--cf <name>`, `--vault
  <path>`, `--sst <path>`. Prints each key (hex) and value (hex) in the CF.
- [x] Write an integration test that opens a `CfRouter`, writes one known key/value
  to each CF, flushes all CFs, then reads back each via `CfRouter::get` and asserts
  byte-exact equality.
- [x] Write a range-scan test: write 5 `ledger` rows with seqs 3,1,5,2,4 (inserted
  out of order); flush; `range(ledger_key(1), ledger_key(6))` returns seqs in
  order `[1,2,3,4,5]` (big-endian sort).
- [x] Write a range-scan test: write 5 `base` rows with known `CxId` values;
  flush; `range(b"\x00".repeat(16), b"\xff".repeat(16))` returns all 5 in
  lexicographic key order.
- [x] Add the `verify_cx_hash_prefix` call in `CfRouter::get` for `CF::Base`: when
  a row is read from SST, verify that the key (CxId bytes) matches the stored
  `full_hash` if the value contains a blake3 digest. (Caller responsibility if
  value encodes the full hash; at minimum, exercise the `verify_cx_hash_prefix`
  error path in the test.)
- [x] Document the `xxd` readback commands in the phase GitHub issue.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: one row per CF round-trips byte-exact via `CfRouter::put` + `flush_cf`
  + `CfRouter::get`.
- [x] unit: `ledger` range scan returns seqs `[1..5]` in ascending order after
  out-of-order inserts and flush.
- [x] proptest: `∀ n in 1..=20 (key, value) pairs` in any CF: after put+flush,
  `range(min, max)` returns all n pairs sorted.
- [x] edge (≥3): (1) empty range scan → empty vec; (2) range that spans two SST
  files returns merged deduped result; (3) key present in both memtable and SST →
  memtable value wins.
- [x] fail-closed: corrupt SST key in `base` CF → `get` returns
  `CALYX_ASTER_CORRUPT_SHARD` (not silently returns None).

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `vault/cf/base/000001.sst` and `vault/cf/ledger/000001.sst` on
  aiwonder at `/home/croyse/calyx/test-vault/`.
- **Readback:**
  ```
  xxd /home/croyse/calyx/test-vault/cf/base/000001.sst | head -4
  xxd /home/croyse/calyx/test-vault/cf/ledger/000001.sst | head -4
  calyx readback --cf base --vault /home/croyse/calyx/test-vault
  calyx readback --cf ledger --vault /home/croyse/calyx/test-vault
  ```
- **Prove:** `base` SST keys are 16-byte CxId values in ascending lexicographic
  order; `ledger` SST keys are 8-byte big-endian seq values in ascending numeric
  order. `calyx readback` output for each CF matches the known written values
  byte-for-byte. Evidence screenshot posted to PH07 GitHub issue.

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] FSV evidence (readback output / screenshot) attached to the PH07 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
