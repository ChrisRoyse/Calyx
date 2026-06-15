# PH47 · T05 — Admission record + integration FSV

| Field | Value |
|---|---|
| **Phase** | PH47 — Lens Proposal (Sufficiency Deficit) |
| **Stage** | S10 — Anneal + Intelligence Objective J |
| **Crate** | `calyx-anneal` |
| **Files** | `crates/calyx-anneal/src/propose/admission_record.rs` (≤500) · `crates/calyx-anneal/tests/fsv_lens_proposal.rs` (≤500) |
| **Depends on** | T01–T04 |
| **Axioms** | A7, A14, A15 |
| **PRD** | `dbprdplans/12 §5`, `dbprdplans/07 §4` |

## Goal

Implement `AdmissionRecord`: structured Ledger entries for every lens proposal
event (`LensAdmitted`, `LensRejected`) with full provenance (deficit map, gate
outcome, sufficiency before/after, candidate description). Also implement the
phase FSV integration test: on a known-insufficient panel, the proposed lens
raises measured `I(panel;anchor)`, and a non-qualifying candidate is rejected
with a logged reason.

## Build (checklist of concrete, code-level steps)

- [ ] `struct LensAdmittedEntry { candidate_desc: String, bits_gain: f64, max_corr: f64, sufficiency_before: f64, sufficiency_after: f64, change_id: ChangeId, ts: LogicalTime }`.
- [ ] `struct LensRejectedEntry { candidate_desc: String, reason: RejectReason, deficit_gap: f64, ts: LogicalTime }`.
- [ ] `fn record_admitted(admitted: &LensAdmittedEntry, ledger: &AnnealLedger) -> Result<LedgerRef, CalyxError>` — serializes and appends to `ledger` CF with `action=LensAdmitted`.
- [ ] `fn record_rejected(rejected: &LensRejectedEntry, ledger: &AnnealLedger) -> Result<LedgerRef, CalyxError>` — appends with `action=LensRejected`.
- [ ] `fn proposal_history(ledger: &AnnealLedger, n: usize) -> Vec<Either<LensAdmittedEntry, LensRejectedEntry>>` — reads last `n` `LensAdmitted`/`LensRejected` entries in order.
- [ ] FSV test `lens_proposal_integration`: (a) create a synthetic vault with panel `[L1]` and a known-insufficient anchor class; (b) measure `sufficiency_before`; (c) call `propose_lens` → `ProposalOutcome { admitted: true, sufficiency_after > sufficiency_before }`; (d) `calyx anneal lens-proposal-log --last 1` shows `LensAdmitted` with exact bits/corr/sufficiency values; (e) craft a second candidate with `bits=0.02` (below threshold) → `ProposalOutcome { admitted: false }`; (f) `lens-proposal-log` shows `LensRejected` with `InsufficientBits`.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] `lens_proposal_integration`: all 6 assertions (a–f) must pass.
- [ ] unit: `record_admitted` then `proposal_history(1)` → returns the `LensAdmittedEntry` byte-exact.
- [ ] unit: `record_rejected` then `proposal_history(1)` → returns the `LensRejectedEntry` byte-exact.
- [ ] edge: `proposal_history(0)` → empty vec; mixed admitted/rejected entries → returned in insertion order.
- [ ] fail-closed: Ledger write fails in `record_admitted` → `CALYX_LEDGER_WRITE_FAIL`; hot-add already applied → caller receives the error and must decide to rollback (documented contract).

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** Ledger `LensAdmitted`/`LensRejected` entries + `I(panel;anchor)` before/after.
- **Readback:** `calyx anneal lens-proposal-log --last 5`; `calyx assay sufficiency --anchor <id>` before and after the test run.
- **Prove:** run `cargo test fsv_lens_proposal` on aiwonder; all assertions green; `lens-proposal-log` shows one `LensAdmitted` (with `sufficiency_after > sufficiency_before`) and one `LensRejected` (with `InsufficientBits`); `calyx assay sufficiency` after the test confirms the higher value. Attach evidence to PH47 GitHub issue.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH47 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
