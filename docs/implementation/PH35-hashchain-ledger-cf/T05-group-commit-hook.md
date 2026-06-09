# PH35 · T05 — Group-commit hook: ledger entry in same WAL batch as data write

| Field | Value |
|---|---|
| **Phase** | PH35 — Hash-chain append-only CF (in group-commit) |
| **Stage** | S7 — Ledger Provenance |
| **Crate** | `calyx-ledger` |
| **Files** | `crates/calyx-ledger/src/group_commit.rs` (≤500) |
| **Depends on** | T03 (this phase) · PH09 (write path group-commit hook points) · PH05 (WAL group-commit) |
| **Axioms** | A15, A16 |
| **PRD** | `dbprdplans/11 §6`, `04 §5` |

## Goal

Wire the ledger appender into PH09's group-commit path so that a `LedgerEntry`
for every constellation mutation is written in the **same WAL record** as the
data it describes. Provenance can therefore never be "added later" and can never
be lost on a crash between the data write and the ledger write — the WAL either
contains both or neither.

**Status:** DONE / FSV-backed by #246 and hardening #345. Evidence roots:
`/home/croyse/calyx/data/fsv-issue246-ledger-group-commit-20260608` and
`/home/croyse/calyx/data/fsv-issue345-ledger-group-commit-atomicity-20260609`.

## Build (checklist of concrete, code-level steps)

- [x] Define `trait LedgerGroupCommitHook` in `group_commit.rs`:
  ```rust
  pub trait LedgerGroupCommitHook: Send + Sync {
      fn on_commit(
          &mut self,
          batch: &mut WriteBatch,
          kind: EntryKind,
          subject: SubjectId,
          payload: Vec<u8>,
          actor: ActorId,
      ) -> Result<LedgerRef>;
  }
  ```
- [x] `struct DefaultLedgerHook { appender: LedgerAppender }` impl stages the
  next entry with `LedgerAppender::prepare`, adds the encoded bytes to the
  batch under the `ledger` CF key `ledger_key(seq)`, then advances the appender
  only through `commit_staged` after the storage batch accepts the row.
- [x] Integrate into PH09's `IngestWriter` (or equivalent group-commit
  coordinator in `calyx-aster`): stage the ledger row before base/slot rows,
  call `commit_rows(...)`, and commit the staged hook state only after the Aster
  batch returns success.
- [x] `kind = EntryKind::Ingest` for constellation creates;
  `kind = EntryKind::Admin` for vault-level operations;
  mapping is defined in `group_commit.rs` as a `const fn ingest_kind_for(op: WriteOp) -> EntryKind`.
- [x] On hook or storage-batch failure, the entire group-commit fails
  atomically: the WAL/data rows are not committed and the in-memory ledger
  appender tip is not advanced.
- [x] `CALYX_LEDGER_GROUP_COMMIT_FAILED` remediation:
  `"ledger hook failed — group-commit rolled back; retry the write"`.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: construct a `WriteBatch`, call `DefaultLedgerHook::on_commit` →
  assert the batch now contains exactly one ledger-CF row under key
  `ledger_key(0)`.
- [x] unit: three sequential `on_commit` calls → assert ledger CF keys are
  `ledger_key(0)`, `ledger_key(1)`, `ledger_key(2)` in the batch (ordered,
  no gaps).
- [x] integration (uses in-process stub WAL): write a constellation via the
  PH09 path with hook attached → replay WAL from `offset=0` → assert the
  ledger CF row is recovered alongside the base/slot CF rows.
- [x] edge (≥3): hook with empty payload → `Ok(LedgerRef)`; hook with
  `store_raw=false` (redaction policy active) → payload stripped; hook called
  with `kind=Erase` → entry written with `kind_code=9`.
- [x] fail-closed: hook returns an error mid-batch → `CALYX_LEDGER_GROUP_COMMIT_FAILED`;
  assert the WAL is not advanced (batch not committed); assert no ledger row
  appears in the CF.
- [x] fail-closed: Aster commit fails after staging a ledger row → exact
  `CALYX_BACKPRESSURE`; assert no logical Ledger CF row, no decoded physical
  ledger row, `snapshot=0`, and hook `next_seq=0`.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** WAL binary file + `ledger` CF rows on aiwonder after an ingest run
- **Readback:**
  1. `xxd $(ls $CALYX_HOME/vault/test/wal/*.bin | tail -1) | head -80` —
     locate the ledger entry bytes; confirm they appear **before** the WAL
     commit marker in the same group-commit record as the base-CF entry.
  2. `calyx readback --vault test --cf ledger --range 0..1` — prints seq=0,
     prev_hash=[0;32], kind=Ingest, entry_hash=<32 bytes>.
- **Prove:** before: no ledger rows; after: ledger row at seq=0 present; WAL
  bytes show both the base-CF write and the ledger-CF write share one commit
  record; crash-recovery test (kill -9 after WAL write, restart) recovers the
  ledger entry alongside the constellation.

**Readback captured for #246:** `ledger-group-commit-readback.json` shows
`before_ledger_row_present=false`, `after_ledger_row_present=true`,
`same_wal_record=true`, `ledger_row_index=0`, `base_row_index=1`,
`ledger_before_base=true`, `entry.seq=0`, zero `prev_hash`, kind `ingest`, and
stored constellation provenance equal to the ledger `entry_hash`. Separate SoT
reads are saved as `04-wal-readback.out`, `05-ledger-cf-readback.out`,
`06-wal-prefix.hex`, and `07-ledger-sst-prefix.hex`.

**Readback captured for #345:** `group-commit-atomicity-readback.json` proves
the injected failure path leaves `before_ledger_row_present=false`,
`after_ledger_row_present=false`, `physical_ledger_rows_after=0`,
`snapshot_after=0`, and hook `next_seq=0`/`store_rows=0`. The success path
proves `ledger_cf_matches_wal_row=true`, `ledger_row_index=0`,
`base_row_index=1`, `ledger_before_base=true`, stored constellation provenance
equals the ledger `entry_hash`, and hook `next_seq=1`/`store_rows=1`. The
aiwonder root manifest is
`f5756e3ed3ab564d013247f8341fde9d56dfe0c690f18572ae9167d9d1d89d0b`.

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] FSV evidence (readback output / screenshot) attached to the PH35 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
