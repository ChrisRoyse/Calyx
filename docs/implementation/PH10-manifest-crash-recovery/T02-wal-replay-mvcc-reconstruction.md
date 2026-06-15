# PH10 · T02 — WAL-replay recovery: reconstruct MVCC from WAL records

| Field | Value |
|---|---|
| **Phase** | PH10 — Manifest + atomic swap + crash recovery |
| **Stage** | S1 — Aster storage core |
| **Crate** | `calyx-aster` |
| **Files** | `crates/calyx-aster/src/manifest/recovery.rs` (≤500), `crates/calyx-aster/src/manifest/mod.rs` (≤500) |
| **Depends on** | T01 (manifest atomic swap), PH09 T02 (WAL write batch format), PH08 T04 (MVCC+CfRouter bridge) |
| **Axioms** | A15, A16 |
| **PRD** | `dbprdplans/04 §7` |

## Goal

Implement `reconstruct_from_recovery(outcome: RecoveryOutcome, cf_router: &mut
CfRouter) -> Result<Seq>`: decode each WAL record from `outcome.wal_records`
using `decode_write_batch`, re-apply each write batch to the `CfRouter` (skipping
rows already in SST from the last flushed manifest), and return the highest
recovered seq. This wires the existing `recover_vault` function into the vault's
cold-open path.

Current status: PH10 is complete for the Stage 1 gate. `recover_vault` reads
the manifest, filters WAL records past `durable_seq`, reports torn tails, and
the durable vault open path reconstructs persisted CF rows from manifest/WAL
state. The `degraded_rebuildable` field is read and propagated when present, but
PH10 never sets it true; that setter/rebuild workflow is deliberately deferred
to PH44 self-heal.

## Build (checklist of concrete, code-level steps)

- [x] In `manifest/recovery.rs`, define `fn reconstruct_from_recovery(outcome:
  RecoveryOutcome, cf_router: &mut CfRouter) -> Result<Seq>`:
  1. For each `ReplayRecord` in `outcome.wal_records`:
     a. `decode_write_batch(&record.payload)?` to get CF rows.
     b. For each row: `cf_router.put(cf, key, value)?`.
  2. Return `outcome.last_recovered_seq`.
- [x] Define `RecoveryState` returned to the vault: `last_seq`, `wal_records_applied`,
  `torn_tail`, `degraded_rebuildable`.
- [x] Read and propagate the `degraded_rebuildable` flag when present; PH44 owns
  setting it true and rebuilding derived CFs.
- [x] Write test: create WAL with 3 records; create MANIFEST at `durable_seq = 0`;
  call `recover_vault` + `reconstruct_from_recovery`; assert `cf_router.get` for
  all 3 records returns the written values.
- [x] Write test: MANIFEST at `durable_seq = 2` with 3 WAL records; only the
  record at seq=3 is re-applied (seq 1 and 2 are already durable in SST).
- [x] Write test: torn WAL tail — recovery applies records before the torn record
  and stops; returns `torn_tail.is_some()`.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: 3-record WAL + durable_seq=0 → all 3 re-applied; `last_recovered_seq=3`.
- [x] unit: 3-record WAL + durable_seq=2 → only seq=3 re-applied.
- [x] unit: torn tail at record 3 → seqs 1+2 applied; torn_tail reported; no panic.
- [x] proptest: for any `n in 1..=20` WAL records with `durable_seq in 0..=n`:
  exactly `n - durable_seq` records are re-applied.
- [x] edge (≥3): (1) `durable_seq = n` (all durable) → 0 re-applied, no error;
  (2) empty WAL → 0 records, no error; (3) `degraded_rebuildable = true` in
  MANIFEST → recovery completes without error, flag propagated.
- [x] fail-closed: `decode_write_batch` on a corrupt WAL payload →
  `CALYX_ASTER_CORRUPT_SHARD`.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** CF directories after recovery at `/home/croyse/calyx/test-vault/cf/`.
- **Readback:**
  ```
  calyx recover --vault /home/croyse/calyx/test-vault
  calyx readback --cf base --vault /home/croyse/calyx/test-vault
  ```
- **Prove:** After `calyx recover`, `calyx readback` shows all rows that were in
  the WAL records after the last durable manifest seq. Rows from before
  `durable_seq` are not duplicated (the SST already has them). Screenshot posted
  to PH10 GitHub issue.

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] FSV evidence (readback output / screenshot) attached to the PH10 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
