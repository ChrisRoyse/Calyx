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

## Build (checklist of concrete, code-level steps)

- [ ] Define `trait LedgerGroupCommitHook` in `group_commit.rs`:
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
- [ ] `struct DefaultLedgerHook { appender: LedgerAppender }` — impl of the
  trait: calls `self.appender.append(kind, subject, payload, actor)`, adds the
  encoded bytes to `batch` under the `ledger` CF key `ledger_key(seq)`.
- [ ] Integrate into PH09's `IngestWriter` (or equivalent group-commit
  coordinator in `calyx-aster`): add an optional `Box<dyn LedgerGroupCommitHook>`
  field; before `batch.commit()` call `hook.on_commit(...)` to add the ledger
  write to the same batch.
- [ ] `kind = EntryKind::Ingest` for constellation creates;
  `kind = EntryKind::Admin` for vault-level operations;
  mapping is defined in `group_commit.rs` as a `const fn ingest_kind_for(op: WriteOp) -> EntryKind`.
- [ ] On hook failure, the entire group-commit fails atomically (the WAL is not
  fsynced); return `CALYX_LEDGER_GROUP_COMMIT_FAILED` (add to error catalog).
- [ ] `CALYX_LEDGER_GROUP_COMMIT_FAILED` remediation:
  `"ledger hook failed — group-commit rolled back; retry the write"`.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: construct a `WriteBatch`, call `DefaultLedgerHook::on_commit` →
  assert the batch now contains exactly one ledger-CF row under key
  `ledger_key(0)`.
- [ ] unit: three sequential `on_commit` calls → assert ledger CF keys are
  `ledger_key(0)`, `ledger_key(1)`, `ledger_key(2)` in the batch (ordered,
  no gaps).
- [ ] integration (uses in-process stub WAL): write a constellation via the
  PH09 path with hook attached → replay WAL from `offset=0` → assert the
  ledger CF row is recovered alongside the base/slot CF rows.
- [ ] edge (≥3): hook with empty payload → `Ok(LedgerRef)`; hook with
  `store_raw=false` (redaction policy active) → payload stripped; hook called
  with `kind=Erase` → entry written with `kind_code=9`.
- [ ] fail-closed: hook returns an error mid-batch → `CALYX_LEDGER_GROUP_COMMIT_FAILED`;
  assert the WAL is not advanced (batch not committed); assert no ledger row
  appears in the CF.

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

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH35 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
