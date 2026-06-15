# PH35 - T03 - LedgerAppender: seq counter + append-only enforcement

| Field | Value |
|---|---|
| **Phase** | PH35 - Hash-chain append-only CF (in group-commit) |
| **Stage** | S7 - Ledger Provenance |
| **Crate** | `calyx-ledger` |
| **Files** | `crates/calyx-ledger/src/append.rs` (<=500) |
| **Depends on** | T02 (this phase) |
| **Axioms** | A15, A16 |
| **PRD** | `dbprdplans/11 section 2`, `11 section 7` |

## Goal

Implement `LedgerAppender`, the single write path for the `ledger` CF. It
maintains the monotonic `seq` counter by recovering from persisted rows, chains
each new entry to the previous `entry_hash`, enforces append-only semantics, and
prohibits tombstones on the `ledger` CF. The appender returns
`LedgerRef { seq, hash }` after each successful write so callers can embed
provenance references.

## Current Implementation

Done in #244. `crates/calyx-ledger/src/append.rs` adds:

- `LedgerCfStore`: minimal append-only row-store contract.
- `LedgerAppender`: recovered `next_seq`, recovered `prev_hash`, injected
  `Clock`, `append(...) -> LedgerRef`, stale-tip detection, and chain recovery
  validation.
- `MemoryLedgerStore`: deterministic unit/proptest store.
- `DirectoryLedgerStore`: disk-backed row store for manual FSV until PH35 T05
  wires Aster's real group-commit CF handle.
- `reject_delete` / `reject_tombstone` fail closed with
  `CALYX_LEDGER_APPEND_ONLY_VIOLATION`.

## Build Checklist

- [x] `struct LedgerAppender` holds `next_seq`, `prev_hash`, a ledger row store,
  and an injected clock.
- [x] `LedgerAppender::open(store, clock)` scans persisted ledger rows, verifies
  contiguous seqs and hash links, and recovers the next seq and tip hash.
- [x] `append(kind, subject, payload, actor)` builds `LedgerEntry`, stamps `ts`
  with the injected clock, encodes bytes, writes a new row, advances state, and
  returns `LedgerRef`.
- [x] Delete/tombstone paths return `CALYX_LEDGER_APPEND_ONLY_VIOLATION`.
- [x] `CALYX_LEDGER_APPEND_ONLY_VIOLATION` is in `calyx-core/src/error.rs` with
  remediation `ledger CF is append-only; deletes and tombstones are forbidden`.
- [x] Seq persistence is row-derived only; no counter file exists.

## Tests

- [x] Unit: empty appender appends three entries with seqs `0,1,2` and correct
  `prev_hash` links.
- [x] Unit: drop/reopen recovers `next_seq` and `prev_hash` from persisted rows.
- [x] Proptest: for `N in 1..=100`, sequential appends preserve the hash chain.
- [x] Edge: single append, reopen with one row, and contiguous recovery checks.
- [x] Fail-closed: seq gaps, stale tip/concurrent append, delete, and tombstone
  all fail with exact structured errors.

## FSV

- **SoT:** physical ledger row files under
  `/home/croyse/calyx/data/fsv-issue244-ledger-appender-20260608/ledger-cf/`.
- **Readback:** ignored test
  `ph35_ledger_appender_aiwonder_fsv` writes five rows, reopens the store, scans
  rows from disk, and writes:
  - `ledger-appender-readback.json`
  - `ledger-range-0-5.txt`
  - `ledger-cf/*.ledger`
- **Prove:** readback shows before row count `0`, after/reopened row count `5`,
  seqs `0..4`, `chain_ok=true`, delete/tombstone error code
  `CALYX_LEDGER_APPEND_ONLY_VIOLATION`, and zero tombstone marker files.

## Done When

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder.
- [x] All `.rs` files <=500 lines.
- [x] FSV evidence attached to #244.
- [x] No anti-pattern: no flattening, no fake trusted state, no frozen-lens
  mutation, and no harness verdict substituted for source-of-truth readback.
