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

## Implementation checkpoint and FSV sign-off (2026-06-10)

`#385` is implemented and FSV-signed-off on aiwonder at
`/home/croyse/calyx/data/fsv-issue385-dedup-audit-20260610-cc9f57b`. The
implementation adds
`crates/calyx-aster/src/dedup/audit.rs`, CLI readbacks in
`crates/calyx-cli/src/dedup_audit_readback.rs`, and the durable readback fixture
`crates/calyx-cli/tests/dedup_audit_readback.rs`.

Important contract details:

- `ReversalToken` includes `vault_id` and `target_cx_id` in addition to
  `ledger_seq_start`, `ledger_seq_end`, and `snapshot_cx_ids`, so undo fails
  closed with `CALYX_DEDUP_WRONG_VAULT` when a token is applied to the wrong
  vault and cannot replay unrelated interleaved merges in the same Ledger range.
- Every `DedupMerge` ledger payload carries a `DedupRestoreSnapshot` containing
  the merged candidate, the pre-merge base row when recurrence changed it, and
  the recurrence occurrence ids that undo must tombstone.
- `dedup_audit` verifies the Ledger CF hash-chain before trusting merge rows
  and reports prior `DedupUndo` entries for the same target CxId.
- `dedup_undo` writes restored base rows, recurrence tombstone rows, and one
  `DedupUndo` Ledger entry in one durable commit. The Ledger payload field is
  named `reversal` instead of `token` so the Ledger redaction policy does not
  reject it as secret-like material; the CLI flag remains `--token`.
- Recurrence undo is logical and append-only: it writes latest tombstone rows
  that recurrence read paths ignore, preserving old bytes for provenance.
- FSV artifact: `dedup-audit-readback.json` BLAKE3
  `4b3031a933685e1d750e52d009c7be33944fb76ea16babb76e830018b966c7a4`.

## Build (checklist of concrete, code-level steps)

- [x] Define `MergeRecord { seq: u64, at: EpochSecs, merged_from: CxId, per_slot_cos: Vec<(SlotId, f32)>, recurrence_signature: bool, anchor_conflict: bool, action: DedupAction }`
- [x] Define `DedupAuditReport { cx_id: CxId, merges: Vec<MergeRecord>, undo_entries: Vec<DedupUndoRecord>, occurrences: Vec<Occurrence>, reversal_token: ReversalToken, anchor_conflict_blocks: Vec<CxId> }`
- [x] Define `ReversalToken { vault_id: VaultId, target_cx_id: CxId, ledger_seq_start: u64, ledger_seq_end: u64, snapshot_cx_ids: Vec<CxId> }` — the vault- and target-bound range of Ledger entries to replay backward to undo all merges
- [x] Implement `dedup_audit(vault: &Vault, cx_id: CxId) -> Result<DedupAuditReport, CalyxError>`:
  - scan the Ledger CF for all entries where `cx_id` is the `into` target; collect `MergeRecord`s
  - read `RecurrenceSeries` from T05 (`series_store.read_series(cx_id)`)
  - scan for `anchor_conflict` entries where `cx_id` is one side
  - compute `ReversalToken` from the span of Ledger seq numbers
  - return full report
- [x] Implement `dedup_undo(vault: &Vault, token: &ReversalToken) -> Result<Vec<CxId>, CalyxError>`:
  - replay the Ledger entries in the token's range backward: reconstruct each pre-merge constellation
  - write reconstructed constellations back to the base CF via WAL group-commit
  - return the list of restored `CxId`s
  - write a `LedgerEntry::DedupUndo { reversal, restored: Vec<CxId> }` entry
- [x] `dedup_undo` is idempotent: re-applying with the same token returns the same result (checks if already undone via Ledger)

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: `dedup_audit` on a CxId with no merges → `merges = []`, `occurrences = []`, `reversal_token.snapshot_cx_ids = [cx_id]`
- [x] unit: after 2 merges into CxId-A, `dedup_audit(A)` → `merges.len() = 2`; each `MergeRecord` has correct `per_slot_cos` values matching what was computed during ingest
- [x] unit: `dedup_undo` after 2 merges → 3 CxIds restored; `read_series(A)` now empty; original 3 CxIds present in CF
- [x] unit: byte-for-byte reversal: `xxd` of restored constellation bytes == `xxd` of original pre-merge bytes
- [x] unit: `dedup_undo` idempotency: calling twice with same token returns same restored CxIds; second call detects `already_undone` in Ledger
- [ ] follow-up/property: `dedup_undo(dedup_audit(cx).reversal_token)` → original constellations restored; a second `dedup_audit` shows the undo entry
- [x] edge: `dedup_undo` on a `ReversalToken` from a different vault → `CALYX_DEDUP_WRONG_VAULT`
- [x] fail-closed: Ledger CF corrupted (bad hash) → `CALYX_LEDGER_CHAIN_BROKEN` propagated; no partial undo

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** Ledger CF rows + CF before/after `dedup_undo`
- **Readback:** after 3 ingests of the same content with `RecurrenceSeries` policy: (1) `calyx readback dedup-audit --vault <dir> --cx-id <CxId>` — print full report; (2) `calyx readback dedup-undo --vault <dir> --token <json>` — apply reversal; (3) `calyx readback cx-list --vault <dir>` — show 3 separate CxIds restored; (4) `calyx readback --cf ledger --vault <dir> --seq <n>` and `calyx readback --cf recurrence --vault <dir>` — show the `DedupUndo` row and recurrence tombstone bytes
- **Prove:** report shows 2 merges with correct per-slot cosines; undo restores 3 CxIds; `xxd` byte-comparison is identical to the first `ingest_at` bytes; Ledger shows `DedupUndo` entry

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] FSV evidence attached to GitHub issue #385
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
