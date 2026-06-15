# PH62 · T07 — Readback extension (CF rows / WAL records / Ledger entries)

| Field | Value |
|---|---|
| **Phase** | PH62 — calyx-cli (vault/lens/ingest/search/readback) |
| **Stage** | S15 — Interfaces: CLI, MCP, Migration |
| **Crate** | `calyx-cli` |
| **Files** | `crates/calyx-cli/src/cmd/readback.rs` (≤500) |
| **Depends on** | T01 (output layer), PH07 (CF key encoding), PH05 (WAL format), PH35 (Ledger CF) |
| **Axioms** | A16 |
| **PRD** | `dbprdplans/14 §2`, `dbprdplans/18 §7` |

## Goal

Extend `calyx readback` beyond the existing `--hex` (arbitrary file) and
`--vault-tree` (directory listing) forms with three new modes that print the
actual bytes of specific Aster structures: `--cf-row` reads a named column-family
row by key, `--wal` reads and decodes WAL records from a WAL segment file, and
`--ledger` reads a Ledger entry by sequence number. These commands are the FSV
gate instrument for every other phase. They print real persisted bytes — never a
harness verdict.

## Build (checklist of concrete, code-level steps)

- [ ] `cmd/readback.rs`: consolidate `readback_hex` and `readback_vault_tree` from
  `main.rs` into this module; re-export them through `cmd/mod.rs`
- [ ] `--cf-row <vault.calyx> --cf <cf-name> --key <hex-key>`: opens the named
  column family directory (e.g. `cf/base/`, `cf/slot_0/`, `cf/anchors/`,
  `cf/ledger/`), reads the SST/data file containing the given big-endian hex key,
  and calls `output::print_hex_dump(0, raw_bytes)` on the full value bytes; fails
  with `CALYX_ASTER_CORRUPT_SHARD` if the key is not found (not a silent miss)
- [ ] `--wal <segment-path>`: opens the named WAL segment file, iterates all
  records, and for each record prints:
  `WAL seq=<u64> group=<u64> len=<u32> crc=<hex8>\n{hex-dump of payload bytes}`;
  a torn tail (incomplete last record) prints `TORN_TAIL seq=<u64>` and exits 0
  (not an error — WAL recovery handles torn tails, readback just shows them)
- [ ] `--ledger <vault.calyx> --seq <u64>`: opens `<vault.calyx>/ledger/` CF,
  reads the entry at the given sequence number, and calls
  `output::print_hex_dump(0, entry_bytes)`; also prints a decoded header line:
  `LEDGER seq=<u64> prev_hash=<hex64> entry_hash=<hex64> kind=<str>`
- [ ] All three modes: `CALYX_CLI_USAGE_ERROR` if vault path is not a valid Calyx
  vault (missing manifest); `CALYX_ASTER_CORRUPT_SHARD` if a CF file has a hash
  mismatch; never fall through to a zero-filled or silent result (A16)
- [ ] Existing `--hex` and `--vault-tree` paths are preserved byte-for-byte
  (regression tests remain green)

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: `--cf-row` on a seeded test vault with a known `CxId` hex key → the
  returned bytes match the bytes written by `ingest` in T03
- [ ] unit: `--wal` on a WAL segment with two committed records and a torn tail →
  output contains two `WAL seq=…` blocks and one `TORN_TAIL` line; no error exit
- [ ] unit: `--ledger --seq 1` after one ingest → header line present with non-zero
  `entry_hash`; hex-dump of entry bytes is non-empty
- [ ] proptest: for any valid CF row written by `VaultStore::put`, `--cf-row` with
  the same key returns bytes that `bincode::deserialize::<Constellation>` succeeds
  on (round-trip)
- [ ] edge: `--cf-row` with a key that does not exist → `CALYX_ASTER_CORRUPT_SHARD`
  on stderr, exit 2; `--wal` on a zero-length file → prints nothing, exits 0;
  `--ledger --seq 0` when seq 0 does not exist → `CALYX_VAULT_ACCESS_DENIED`
- [ ] fail-closed: corrupt CRC in a WAL record → `CALYX_ASTER_TORN_WAL` with
  message showing the bad seq; never silently skips the corrupt record

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** the raw on-disk files in `<vault.calyx>/cf/`, `<vault.calyx>/wal/`,
  `<vault.calyx>/ledger/` on aiwonder after a real vault workflow
- **Readback:**
  - `calyx readback --cf-row <vault.calyx> --cf base --key <cx_id_hex>` → hex
    dump of the base CF row; `xxd <vault.calyx>/cf/base/<cx_id_hex>` on aiwonder
    must match byte-for-byte
  - `calyx readback --wal <vault.calyx>/wal/00000001.wal` → WAL records printed
    with seq numbers and hex payloads; torn tail if present shown as `TORN_TAIL`
  - `calyx readback --ledger <vault.calyx> --seq 1` → Ledger entry header +
    hex dump; `entry_hash` matches the hash printed by `verify-chain`
- **Prove:** the hex-dump bytes from `--cf-row` are identical to the bytes from
  direct `xxd` on the same file (byte-exact, not a harness assertion); the WAL
  record count from `--wal` matches the number of committed ingests; the Ledger
  `prev_hash` at seq N matches the `entry_hash` at seq N-1 (chain integrity)

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH62 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
