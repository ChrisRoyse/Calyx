# PH43 - T05 - Ledger `kind=Anneal` writer

| Field | Value |
|---|---|
| **Phase** | PH43 - Tripwires + Shadow-First + Reversible/Rollback |
| **Stage** | S10 - Anneal + Intelligence Objective J |
| **Crate** | `calyx-anneal` |
| **Files** | `crates/calyx-anneal/src/ledger_anneal.rs` (329 lines) |
| **Depends on** | T03 (RollbackStore provides ChangeId + artifact snapshots) |
| **Axioms** | A14, A15 |
| **PRD** | `dbprdplans/12 section 6`, `dbprdplans/27 section 4` |
| **Issue** | #398 |
| **Status** | Done |

## Goal

Implement `AnnealLedger`: writes every Anneal promotion, revert, proposal,
park, recalibration, mistake update, and autotune A/B event as a hash-chained
Ledger entry with `kind=Anneal`.

PH35 has landed. T05 therefore uses the existing `calyx-ledger`
`LedgerAppender` and `EntryKind::Anneal` instead of a side format or direct CF
encoding. The Aster adapter writes the prepared ledger bytes into the real
`ledger` CF with big-endian sequence keys.

## Build

- [x] `enum AnnealLedgerAction { Promote, Revert, Propose, Park, Recalibrate, MistakeUpdate, AutotuneAB }`
  covers every Anneal event type without colliding with the existing
  `AnnealAction` shadow-execution trait.
- [x] `struct AnnealLedgerEntry` records `action`, `change_id`,
  `artifact_id`, `prior_ptr_hash`, `candidate_ptr_hash`, `metrics`, `ts`,
  `description`, and `prev_hash`.
- [x] Serialized payload includes `"kind":"Anneal"` and
  `"tag":"anneal_event_v1"` as JSON so existing ledger audit/readback surfaces
  can decode it and the redaction policy can inspect it.
- [x] `artifact_id` is used instead of `artifact_key`; `_key` fields are
  intentionally rejected by the ledger secret scanner.
- [x] `AnnealLedger<S,C>` wraps `LedgerAppender<S,C>` and returns
  `LedgerRef { seq, hash }` from `write`.
- [x] `AsterAnnealLedgerStore` adapts an `AsterVault` to `LedgerCfStore`, using
  `cf::ledger_key(seq)` for physical CF rows.
- [x] `read_recent`, `read_recent_with_refs`, `find_by_change_id`, and
  `find_by_change_id_with_ref` read back only `EntryKind::Anneal` entries.
- [x] `write` canonicalizes `prev_hash` to the current ledger tip and fails
  closed with `CALYX_LEDGER_CHAIN_BROKEN` if the caller supplies a mismatched
  hash.
- [x] Oversized payloads fail with `CALYX_LEDGER_ENTRY_TOO_LARGE` before any CF
  mutation.

## Tests

- [x] Unit: write a `Promote` entry then `read_recent(1)` deserializes to the
  same entry and `LedgerRef`.
- [x] Unit: write `Promote` then `Revert`; `read_recent(2)` returns both in
  order and `find_by_change_id` returns the expected event.
- [x] Unit: repeated change-id lookup returns the latest matching event.
- [x] Proptest: write sequences preserve insertion order with monotonic ledger
  sequence numbers.
- [x] Edge: CF unavailable propagates `CALYX_ASTER_CF_UNAVAILABLE`.
- [x] Edge: empty `description` succeeds and reading from an empty CF returns
  an empty vec.
- [x] Fail-closed: oversized entry returns `CALYX_LEDGER_ENTRY_TOO_LARGE` and
  writes no row.
- [x] Fail-closed: mismatched `prev_hash` returns
  `CALYX_LEDGER_CHAIN_BROKEN` and writes no row.

## FSV

Source of truth: Aster vault `ledger` CF rows under
`/home/croyse/calyx/data/fsv-issue398-anneal-ledger-20260610-1905/vault`.

Evidence root:
`/home/croyse/calyx/data/fsv-issue398-anneal-ledger-20260610-1905`

Readbacks captured:

- `anneal-ledger-readback.json` - before/after state, decoded rows, and edge
  transitions.
- `audit-anneal.json` - `calyx audit --vault <vault> --kind anneal`.
- `scan-ledger.jsonl` - `calyx scan --cf ledger --vault <vault>`.
- `raw-ledger-seq0.txt`, `raw-ledger-seq1.txt`, `raw-ledger-seq2.txt` -
  `calyx readback --cf ledger --vault <vault> --seq <n>` raw CF values.
- `verify-chain.txt` - `CHAIN_INTACT count=3`.
- `merkle-root.txt` -
  `fc215a9d005d5a0f1486aeb7d6fc6f2d94bdb2e7cac38b14142b17e565e27e7e`.
- `physical-files.txt` and `vault-tree.txt` - durable WAL/SST file listing.
- `xxd-ledger-sst-head.txt` - direct hex read of the physical ledger SST
  showing `anneal_event_v1`.
- `MANUAL_BLAKE3SUMS.txt` - manually generated and verified BLAKE3 manifest.

Key observed values:

- Before happy path: no `ledger` CF rows.
- After happy path: seq 0 `action=promote`, seq 1 `action=revert`.
- Revert row: `change_id=398001`,
  `prior_ptr_hash=1111111111111111111111111111111111111111111111111111111111111111`,
  `candidate_ptr_hash=2222222222222222222222222222222222222222222222222222222222222222`.
- Edge empty description: row count 2 -> 3 and payload `description=""`.
- Edge oversized payload: row count stayed 3, code
  `CALYX_LEDGER_ENTRY_TOO_LARGE`.
- Edge mismatched `prev_hash`: row count stayed 3, code
  `CALYX_LEDGER_CHAIN_BROKEN`.
- Edge empty CF: before `[]`, `read_recent(10)=[]`, after `[]`.

## Gates

- [x] `cargo fmt --check`
- [x] `bash scripts/linecount.sh`
- [x] `git diff --check`
- [x] `cargo check -p calyx-anneal --quiet`
- [x] `cargo test -p calyx-anneal --test ledger_anneal --quiet`
- [x] `cargo test -p calyx-anneal --test ledger_anneal_fsv --quiet`
- [x] `cargo test -p calyx-anneal --quiet`
- [x] `cargo clippy -p calyx-anneal --tests --quiet -- -D warnings`
- [x] `cargo check -p calyx-cli --quiet`
- [x] `cargo test -p calyx-cli --quiet`

## Done

T05 is complete. T06 can now consume `AnnealLedger` for the integrated
bad-change auto-revert scenario.
