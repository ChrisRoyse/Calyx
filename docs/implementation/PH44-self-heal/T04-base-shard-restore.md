# PH44 · T04 — Base-shard restore path (fail-closed + restic alert)

| Field | Value |
|---|---|
| **Phase** | PH44 — Self-Heal (Rebuild Derived, Degrade Flags) |
| **Stage** | S10 — Anneal + Intelligence Objective J |
| **Crate** | `calyx-anneal` |
| **Files** | `crates/calyx-anneal/src/heal/restore.rs` (≤500) |
| **Depends on** | T01 (DegradeRegistry), T02 (ChecksumDetector fires base-corrupt fault) |
| **Axioms** | A16, A15 |
| **PRD** | `dbprdplans/12 §2` |

## Goal

Handle the case where a base shard (the ground-truth data, not a derived
structure) is corrupt: fail reads on the affected range closed with
`CALYX_ASTER_BASE_CORRUPT`, alert the operator (structured log + Ledger entry),
and optionally trigger a restore from restic backup or ZFS snapshot — gated on
explicit vault config (`auto_restore: true`). The self-heal path never silently
degrades a base shard; data integrity is paramount.

## Build (checklist of concrete, code-level steps)

- [ ] `struct BaseShard { shard_id: ShardId, cf_range: KeyRange, checksum: [u8;32] }` — metadata for each base shard; checksums stored in `anneal_checksums` CF.
- [ ] `fn verify_base_shards(vault: &Vault, clock: &dyn Clock) -> Vec<BaseFaultEvent>` — iterates base CF ranges, computes SHA-256 of each shard's byte content, compares to stored checksum; returns `BaseFaultEvent::Corrupt { shard_id, expected, actual }` for any mismatch.
- [ ] `fn fail_reads_on_range(vault: &mut Vault, shard_id: ShardId) -> Result<(), CalyxError>` — inserts a `ReadBarrier` for the affected key range in the MVCC read path; reads crossing the barrier return `CALYX_ASTER_BASE_CORRUPT` with remediation message pointing to the shard_id.
- [ ] `fn alert_operator(event: &BaseFaultEvent, ledger: &AnnealLedger)` — writes a Ledger entry `action=BaseCorruptAlert` with `shard_id`, `checksum_expected`, `checksum_actual`, `ts`; also writes to a structured log file `$CALYX_HOME/vault/alerts.jsonl`.
- [ ] `fn attempt_restore(shard_id: ShardId, config: &RestoreConfig) -> Result<RestoreOutcome, CalyxError>` — if `config.auto_restore=true`, invokes `restic restore` or `zfs clone` for the latest snapshot covering the shard; if `auto_restore=false` returns `RestoreOutcome::OperatorRequired`.
- [ ] After successful restore: re-verify checksum, update `anneal_checksums` CF, call `DegradeRegistry::set_health(Ok)`, write Ledger `action=BaseRestored`.
- [ ] No base-shard mutation in any Anneal code path; base shards are read-only to Anneal (writes go through Aster WAL only).

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: inject a `BaseFaultEvent::Corrupt` for `shard_0`; call `fail_reads_on_range`; attempt a read on a key in the range → `CALYX_ASTER_BASE_CORRUPT`; read on a key outside the range → succeeds.
- [ ] unit: `alert_operator` called → Ledger contains `action=BaseCorruptAlert` with correct shard_id; `alerts.jsonl` contains a line with the shard_id and checksums.
- [ ] proptest: for any `shard_id`, `fail_reads_on_range` then remove the barrier → reads succeed again (barrier is reversible).
- [ ] edge: all shards corrupt → all reads fail closed (no silent zero-fill); `auto_restore=false` → `OperatorRequired` without attempting restore; restore fails (restic error) → `CALYX_ANNEAL_RESTORE_FAILED`; Ledger entry written.
- [ ] fail-closed: `alerts.jsonl` write fails → still writes Ledger entry; error surfaced but does not prevent barrier placement.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `anneal_checksums` CF, `alerts.jsonl`, Ledger `BaseCorruptAlert` entry, read barrier in effect.
- **Readback:** `cat $CALYX_HOME/vault/alerts.jsonl | tail -3`; `calyx readback ledger --kind Anneal --action BaseCorruptAlert --last 1`; attempt a read on the affected key range → confirm `CALYX_ASTER_BASE_CORRUPT`.
- **Prove:** corrupt a base shard checksum entry; run `verify_base_shards`; confirm `alerts.jsonl` has the entry; confirm the affected range returns `CALYX_ASTER_BASE_CORRUPT`; confirm base CF bytes are untouched (no data deletion by Anneal).

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH44 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
