# PH35 · T03 — `LedgerAppender`: seq counter + append-only enforcement

| Field | Value |
|---|---|
| **Phase** | PH35 — Hash-chain append-only CF (in group-commit) |
| **Stage** | S7 — Ledger Provenance |
| **Crate** | `calyx-ledger` |
| **Files** | `crates/calyx-ledger/src/append.rs` (≤500) |
| **Depends on** | T02 (this phase) |
| **Axioms** | A15, A16 |
| **PRD** | `dbprdplans/11 §2`, `11 §7` |

## Goal

Implement `LedgerAppender` — the single write path for the `ledger` CF. It
maintains the monotonic `seq` counter (persisted, recovered on restart), chains
each new entry to the previous `entry_hash`, enforces append-only semantics
(no update, no delete), and prohibits LSM tombstones on the `ledger` CF. The
appender returns a `LedgerRef { seq, hash }` (reusing the `calyx-core` type)
after each successful write so callers can embed provenance references in their
own structs.

## Build (checklist of concrete, code-level steps)

- [ ] `struct LedgerAppender` — holds: `next_seq: u64`, `prev_hash: [u8; 32]`,
  a write-batch handle to the `ledger` CF, and a `Clock` trait reference.
- [ ] `fn LedgerAppender::open(cf_handle) -> Result<Self>` — recovers
  `next_seq` by scanning the last row in the `ledger` CF (big-endian `seq`
  key, so `last()` = highest seq); recovers `prev_hash` from that row's
  `entry_hash`; if CF is empty, `next_seq = 0`, `prev_hash = [0u8; 32]`.
- [ ] `fn append(&mut self, kind: EntryKind, subject: SubjectId, payload: Vec<u8>, actor: ActorId) -> Result<LedgerRef>` —
  builds the `LedgerEntry` (assigns `seq`, chains `prev_hash`, stamps `ts` via
  injected `Clock`), encodes it, writes it to the CF write-batch, increments
  `next_seq`, updates `prev_hash`; returns `LedgerRef { seq, hash: entry_hash }`.
- [ ] Append-only guard: `fn reject_delete(cf: ColumnFamily) -> Result<()>` —
  any call that would issue a tombstone/delete on the `ledger` CF must return
  `CALYX_LEDGER_APPEND_ONLY_VIOLATION` (add to error catalog with remediation
  `"ledger CF is append-only; deletes and tombstones are forbidden"`).
- [ ] `CALYX_LEDGER_APPEND_ONLY_VIOLATION` added to `calyx-core/src/error.rs`.
- [ ] `seq` persistence: after `append`, the new seq is durably encoded in the
  written row — no separate counter file; `open` always recomputes from CF scan.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: open an empty appender → append 3 entries (Ingest, Measure, Assay) →
  assert seq = 0,1,2; assert `prev_hash` of entry[1] == `entry_hash` of entry[0];
  assert `prev_hash` of entry[2] == `entry_hash` of entry[1].
- [ ] unit: simulate crash-recovery — write 5 entries, drop appender, re-open →
  assert `next_seq == 5` and `prev_hash` matches entry[4]'s `entry_hash`.
- [ ] proptest: for N ∈ 1..=100 sequential appends, the chain is intact
  (`entry[i].prev_hash == entry[i-1].entry_hash` for all i > 0).
- [ ] edge (≥3): single append to empty CF; reopen on CF with exactly 1 entry;
  reopen on CF with 1000 entries (seq skip is not allowed — must be contiguous).
- [ ] fail-closed: attempt to call a delete-path on the ledger CF →
  `CALYX_LEDGER_APPEND_ONLY_VIOLATION`; attempt to append with a stale
  `prev_hash` (simulated concurrent write) → detected at verify step.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `ledger` CF rows on disk after running the smoke test
- **Readback:** `calyx readback --vault test --cf ledger --range 0..5`
  prints each row; confirm `seq` values 0,1,2,3,4 in order; confirm that
  `prev_hash` of row N matches `entry_hash` of row N-1 byte-for-byte.
- **Prove:** before: no appender exists; after: 5 rows present in CF; chain
  links byte-exact (verify by eyeball or script); no tombstone markers in the
  `ledger` CF scan output (absence proof — `xxd` shows no `0xFF` delete markers).

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH35 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
