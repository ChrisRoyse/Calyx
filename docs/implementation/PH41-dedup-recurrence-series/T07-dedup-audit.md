# PH41 · T07 — `dedup_audit` (per-slot cos, reversible, Ledger-logged)

| Field | Value |
|---|---|
| **Phase** | PH41 — DedupPolicy TctCosine + Recurrence Series + Signature |
| **Stage** | S9 — Temporal & Dedup |
| **Crate** | `calyx-aster` |
| **Files** | `crates/calyx-aster/src/dedup/audit.rs` (≤500) |
| **Depends on** | T06 (this phase) · PH35 (Ledger hash-chain CF) |
| **Axioms** | A28, A15 |
| **PRD** | `dbprdplans/25 §5`, `dbprdplans/25 §8` |

## Goal

Implement `dedup_audit(vault, cx_id) -> Result<DedupAuditReport, CalyxError>` —
the read-path function that returns the complete dedup history for a constellation:
all merges that collapsed other CxIds into this one, the per-slot cosine scores
that triggered each merge, anchor-conflict blocks, recurrence series occurrences,
and a reversal token. The reversal token can be passed to `dedup_undo(vault,
token)` to reconstruct the original pre-merge constellation(s) byte-for-byte.
All merge events are Ledger-logged; `dedup_audit` reads the Ledger CF.

## Build (checklist of concrete, code-level steps)

- [ ] Define `MergeRecord { seq: u64, at: EpochSecs, merged_from: CxId, per_slot_cos: Vec<(SlotId, f32)>, recurrence_signature: bool, anchor_conflict: bool, action: DedupAction }`
- [ ] Define `DedupAuditReport { cx_id: CxId, merges: Vec<MergeRecord>, occurrences: Vec<Occurrence>, reversal_token: ReversalToken, anchor_conflict_blocks: Vec<CxId> }`
- [ ] Define `ReversalToken { ledger_seq_start: u64, ledger_seq_end: u64, snapshot_cx_ids: Vec<CxId> }` — the range of Ledger entries to replay backward to undo all merges
- [ ] Implement `dedup_audit(vault: &Vault, cx_id: CxId) -> Result<DedupAuditReport, CalyxError>`:
  - scan the Ledger CF for all entries where `cx_id` is the `into` target; collect `MergeRecord`s
  - read `RecurrenceSeries` from T05 (`series_store.read_series(cx_id)`)
  - scan for `anchor_conflict` entries where `cx_id` is one side
  - compute `ReversalToken` from the span of Ledger seq numbers
  - return full report
- [ ] Implement `dedup_undo(vault: &mut Vault, token: &ReversalToken) -> Result<Vec<CxId>, CalyxError>`:
  - replay the Ledger entries in the token's range backward: reconstruct each pre-merge constellation
  - write reconstructed constellations back to the base CF via WAL group-commit
  - return the list of restored `CxId`s
  - write a `LedgerEntry::DedupUndo { token, restored: Vec<CxId> }` entry
- [ ] `dedup_undo` is idempotent: re-applying with the same token returns the same result (checks if already undone via Ledger)

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: `dedup_audit` on a CxId with no merges → `merges = []`, `occurrences = []`, `reversal_token.snapshot_cx_ids = [cx_id]`
- [ ] unit: after 2 merges into CxId-A, `dedup_audit(A)` → `merges.len() = 2`; each `MergeRecord` has correct `per_slot_cos` values matching what was computed during ingest
- [ ] unit: `dedup_undo` after 2 merges → 3 CxIds restored; `read_series(A)` now empty; original 3 CxIds present in CF
- [ ] unit: byte-for-byte reversal: `xxd` of restored constellation bytes == `xxd` of original pre-merge bytes
- [ ] unit: `dedup_undo` idempotency: calling twice with same token returns same restored CxIds; second call detects `already_undone` in Ledger
- [ ] proptest: `dedup_undo(dedup_audit(cx).reversal_token)` → original constellations restored; a second `dedup_audit` shows the undo entry
- [ ] edge: `dedup_undo` on a `ReversalToken` from a different vault → `CALYX_DEDUP_WRONG_VAULT`
- [ ] fail-closed: Ledger CF corrupted (bad hash) → `CALYX_LEDGER_CHAIN_BROKEN` propagated; no partial undo

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** Ledger CF rows + CF before/after `dedup_undo`
- **Readback:** after 3 ingests of the same content with `RecurrenceSeries` policy: (1) `calyx readback dedup-audit <CxId>` — print full report; (2) `calyx readback dedup-undo --token <token>` — apply reversal; (3) `calyx readback cx-list` — show 3 separate CxIds restored; (4) `xxd` one restored constellation to compare with original ingest bytes
- **Prove:** report shows 2 merges with correct per-slot cosines; undo restores 3 CxIds; `xxd` byte-comparison is identical to the first `ingest_at` bytes; Ledger shows `DedupUndo` entry

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH41 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
