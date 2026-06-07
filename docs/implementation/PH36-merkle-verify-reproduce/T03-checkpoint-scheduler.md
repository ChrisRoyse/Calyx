# PH36 · T03 — Checkpoint scheduler: periodic Merkle root written as Admin entry

| Field | Value |
|---|---|
| **Phase** | PH36 — Merkle checkpoints + verify_chain + reproduce() |
| **Stage** | S7 — Ledger Provenance |
| **Crate** | `calyx-ledger` |
| **Files** | `crates/calyx-ledger/src/checkpoint.rs` (≤500) |
| **Depends on** | T01, T02 (this phase) |
| **Axioms** | A15 |
| **PRD** | `dbprdplans/11 §2`, `11 §6` |

## Goal

Periodically materialise a Merkle root over the last `checkpoint_interval`
ledger entries and write it as a `kind=Admin` ledger entry (with a structured
payload identifying it as a checkpoint, the range it covers, and the root
hash). Checkpoints are themselves hash-chained into the ledger, making the
ledger self-attestating. They also serve as fast-skip anchor points for
`verify_chain`: a verified checkpoint's root allows verification to skip to the
next checkpoint boundary rather than re-hashing all prior entries.

## Build (checklist of concrete, code-level steps)

- [ ] `struct CheckpointConfig { interval_entries: u64, sign_key: Option<[u8; 32]> }` —
  default `interval_entries = 1000`.
- [ ] `struct CheckpointScheduler { config: CheckpointConfig, next_checkpoint_at: u64 }` —
  tracks when the next checkpoint is due.
- [ ] `fn CheckpointScheduler::should_checkpoint(&self, current_seq: u64) -> bool` —
  returns `true` when `current_seq >= self.next_checkpoint_at`.
- [ ] `fn CheckpointScheduler::write_checkpoint(&mut self, appender: &mut LedgerAppender, cf_reader: &dyn LedgerCfReader, range_end_seq: u64) -> Result<LedgerRef>` —
  computes `merkle_root(range_start..range_end_seq)`, optionally signs it,
  builds a `CheckpointPayload { range_start, range_end, root, signature, signer_pubkey }`,
  calls `appender.append(EntryKind::Admin, SubjectId::System, payload, ActorId::System)`,
  updates `next_checkpoint_at = range_end_seq + interval_entries`.
- [ ] `struct CheckpointPayload` — serde JSON; tag `"checkpoint_v1"` in the
  `payload` bytes so it can be distinguished from other Admin entries.
- [ ] Integrate `CheckpointScheduler::should_checkpoint` into `LedgerGroupCommitHook::on_commit`
  — after each append, if the scheduler fires, write the checkpoint in the same
  commit batch.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: configure `interval_entries=5`; append 15 entries; assert 3
  checkpoint entries were written (at seq=5, 11, 17 — checkpoint itself
  consumes a seq slot; exact seqs depend on impl, assert 3 checkpoints present).
- [ ] unit: decode a checkpoint entry payload → assert it is tagged
  `"checkpoint_v1"`, carries the correct `range_start`, `range_end`, and a
  non-zero `root`.
- [ ] unit: checkpoint root matches the result of calling `merkle_root` directly
  over the same range (byte-exact).
- [ ] edge (≥3): `interval_entries=1` (checkpoint every entry); `interval_entries=u64::MAX`
  (never fires during test); zero entries after last checkpoint → no spurious
  checkpoint written; sign key `None` → `signature=None` in payload.
- [ ] fail-closed: `merkle_root` fails (I/O error) → checkpoint is skipped and
  `CALYX_ASTER_IO_ERROR` propagated (not silently swallowed); no partial
  checkpoint entry written.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `ledger` CF rows including checkpoint entries after running the smoke
  ingest with checkpointing enabled
- **Readback:** `calyx scan --cf ledger | jq 'select(.kind=="Admin")' | head -3` —
  prints the first 3 Admin entries; confirm each has `"checkpoint_v1"` tag,
  a `range_start`, a `range_end`, and a 32-byte `root` hex string.
- **Prove:** before: no Admin checkpoint entries; after: one checkpoint per
  `interval_entries` appends; the checkpoint `root` byte-matches the direct
  `calyx merkle-root --range <start>..<end>` output.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH36 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
