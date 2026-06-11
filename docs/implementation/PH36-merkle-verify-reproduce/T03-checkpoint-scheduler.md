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

## Status (2026-06-09)

DONE / FSV-signed-off in #251. Implementation lives in
`crates/calyx-ledger/src/checkpoint.rs`, with staged checkpoint chaining in
`group_commit.rs`, Aster cadence configuration through `VaultOptions`, and
decoded CLI readback via `calyx scan --cf ledger --vault <dir>`.

FSV root:
`/home/croyse/calyx/data/fsv-issue251-checkpoint-scheduler-20260609`.
Manual readback files under `manual-readback/` include decoded Admin
checkpoint JSON, direct Merkle root reads for each checkpoint range, raw
Ledger CF bytes, WAL bytes, vault tree, and SHA256 manifests.

## Build (checklist of concrete, code-level steps)

- [x] `struct CheckpointConfig { interval_entries: u64, sign_key: Option<[u8; 32]> }` —
  default `interval_entries = 1000`.
- [x] `struct CheckpointScheduler { config: CheckpointConfig, next_checkpoint_at: u64 }` —
  tracks when the next checkpoint is due and the next range start.
- [x] `fn CheckpointScheduler::should_checkpoint(&self, current_seq: u64) -> bool` —
  returns `true` when `current_seq >= self.next_checkpoint_at`.
- [x] `fn CheckpointScheduler::prepare_checkpoint_after(...) -> Result<PreparedLedgerEntry>` —
  computes `merkle_root(range_start..range_end_seq)`, optionally signs it,
  builds `CheckpointPayload { range_start, range_end, root, signature, signer_pubkey }`,
  and stages a chained `EntryKind::Admin` row after the triggering ingest row.
- [x] `struct CheckpointPayload` — serde JSON; tag `"checkpoint_v1"` in the
  `payload` bytes so it can be distinguished from other Admin entries.
- [x] Integrate `CheckpointScheduler::should_checkpoint` into
  `DefaultLedgerHook::stage_with_checkpoints` so checkpoint rows are staged in
  the same durable commit batch as the data row. Public callers must commit
  staged rows only after storage commit succeeds.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: configure `interval_entries=5`; append 15 entries; assert 3
  checkpoint entries were written (at seq=5, 11, 17 — checkpoint itself
  consumes a seq slot; exact seqs depend on impl, assert 3 checkpoints present).
- [x] unit: decode a checkpoint entry payload → assert it is tagged
  `"checkpoint_v1"`, carries the correct `range_start`, `range_end`, and a
  non-zero `root`.
- [x] unit: checkpoint root matches the result of calling `merkle_root` directly
  over the same range (byte-exact).
- [x] edge (≥3): `interval_entries=1` (checkpoint every entry); `interval_entries=u64::MAX`
  (never fires during test); zero entries after last checkpoint → no spurious
  checkpoint written; sign key `None` → `signature=None` in payload.
- [x] fail-closed: `merkle_root` fails (I/O/read error) → checkpoint is skipped
  and the underlying error is propagated (not silently swallowed); no partial
  checkpoint entry written.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `ledger` CF rows including checkpoint entries after running the smoke
  ingest with checkpointing enabled
- **Readback:** `calyx scan --cf ledger --vault <dir> | jq -c 'select(.kind=="Admin")'` —
  prints the first 3 Admin entries; confirm each has `"checkpoint_v1"` tag,
  a `range_start`, a `range_end`, and a 32-byte `root` hex string.
- **Prove:** before: no Admin checkpoint entries; after: one checkpoint per
  `interval_entries` appends; the checkpoint `root` byte-matches the direct
  `calyx merkle-root --range <start>..<end>` output.

Issue #251 FSV proved this with three signed Admin rows:
seq 3 covers `0..3`, seq 7 covers `4..7`, seq 11 covers `8..11`; direct
`calyx merkle-root --vault` reads returned matching roots for all three.

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] FSV evidence (readback output / screenshot) attached to the PH36 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
