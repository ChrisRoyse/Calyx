# PH43 · T05 — Ledger `kind=Anneal` writer

| Field | Value |
|---|---|
| **Phase** | PH43 — Tripwires + Shadow-First + Reversible/Rollback |
| **Stage** | S10 — Anneal + Intelligence Objective J |
| **Crate** | `calyx-anneal` |
| **Files** | `crates/calyx-anneal/src/ledger_anneal.rs` (≤500) |
| **Depends on** | T03 (RollbackStore provides ChangeId + ArtifactSnapshot for log entries) |
| **Axioms** | A14, A15 |
| **PRD** | `dbprdplans/12 §6`, `dbprdplans/27 §4` |

## Goal

Implement `AnnealLedger`: writes every Anneal promotion, revert, proposal, park,
and recalibration as a Ledger entry with `kind=Anneal`. Entries are forward-
compatible with the hash-chain Ledger added in PH35; until PH35 lands the writer
appends to the `ledger` CF directly using the same record format. Every change
in Anneal is fully auditable: what changed, why, which metrics triggered, which
change_id, timestamp (logical clock), and whether it was a promotion or a revert.

## Build (checklist of concrete, code-level steps)

- [ ] `enum AnnealAction { Promote, Revert, Propose, Park, Recalibrate, MistakeUpdate, AutotuneAB }` — covers every Anneal event type.
- [ ] `struct AnnealLedgerEntry { kind: "Anneal", action: AnnealAction, change_id: ChangeId, artifact_key: String, prior_ptr_hash: [u8;32], candidate_ptr_hash: [u8;32], metrics: MetricSnapshot, ts: LogicalTime, description: String }` — serialized as CBOR/msgpack in the Ledger CF value.
- [ ] `struct AnnealLedger { cf: LedgerCf, clock: Arc<dyn Clock> }` wrapping the Aster CF handle.
- [ ] `fn write(&self, entry: AnnealLedgerEntry) -> Result<LedgerRef, CalyxError>` — appends to the `ledger` CF under a monotonic seq key; returns a `LedgerRef` (seq + hash) that can be embedded in rollback snapshots and shadow verdicts.
- [ ] `fn read_recent(&self, n: usize) -> Vec<AnnealLedgerEntry>` — reads the last `n` entries from the CF; used by `calyx anneal ledger --last N`.
- [ ] `fn find_by_change_id(&self, id: ChangeId) -> Option<AnnealLedgerEntry>` — point lookup for FSV / rollback audit.
- [ ] Every entry includes a `prior_ptr_hash` and `candidate_ptr_hash` (SHA-256 of the artifact bytes) so byte-level before/after is auditable without re-reading the artifact.
- [ ] Forward-compatible with PH35 hash-chain: leave a `prev_hash: Option<[u8;32]>` field; set to `None` until PH35 wires the chain.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: write a `Promote` entry then `read_recent(1)` → deserializes to equal entry; `change_id`, `action`, `metrics` match exactly.
- [ ] unit: write `Promote` then `Revert` → `read_recent(2)` returns both in order; `find_by_change_id` returns the right entry for each.
- [ ] proptest: any sequence of writes followed by `read_recent(N)` returns entries in insertion order with monotonically increasing seq keys.
- [ ] edge: CF unavailable → `CALYX_ASTER_CF_UNAVAILABLE`; write with empty `description` → still succeeds (description is optional); reading from empty CF → returns empty vec, no panic.
- [ ] fail-closed: CBOR serialization of an oversized entry → `CALYX_LEDGER_ENTRY_TOO_LARGE`; never silently truncates.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `ledger` CF rows with `kind=Anneal`.
- **Readback:** `calyx readback ledger --kind Anneal --last 10` (or `xxd` the CF at the seq key range) — prints 10 most-recent Anneal entries with `action`, `change_id`, `metrics`, pointer hashes.
- **Prove:** trigger a shadow revert (from T02); read the Ledger; confirm a `Revert` entry with the correct `change_id`, the correct `prior_ptr_hash`, and `action=Revert` is present. The entry must be absent before the revert and present after — delta proves the log is live.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH43 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
