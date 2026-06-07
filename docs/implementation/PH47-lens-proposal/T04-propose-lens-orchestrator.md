# PH47 · T04 — `propose_lens` orchestrator (hot-add + re-measure)

| Field | Value |
|---|---|
| **Phase** | PH47 — Lens Proposal (Sufficiency Deficit) |
| **Stage** | S10 — Anneal + Intelligence Objective J |
| **Crate** | `calyx-anneal` |
| **Files** | `crates/calyx-anneal/src/propose/propose_lens.rs` (≤500) |
| **Depends on** | T01, T02, T03 (deficit → synth → gate pipeline) |
| **Axioms** | A7, A14, A15 |
| **PRD** | `dbprdplans/12 §5`, `dbprdplans/27 §3` |

## Goal

Implement the top-level `propose_lens(anchor, vault) -> ProposalOutcome` function
that orchestrates the full pipeline: localize deficit → synthesize candidate →
profile → gate → hot-add via Registry (PH20) → re-measure `I(panel;anchor)` →
confirm sufficiency rose → Ledger-log. If hot-add via Registry fails or re-
measurement shows no improvement, the proposal is rolled back (tripwire-guarded
via PH43 substrate). No data deleted at any step.

## Build (checklist of concrete, code-level steps)

- [ ] `struct ProposalOutcome { candidate: CandidateLens, gate_outcome: GateOutcome, sufficiency_before: f64, sufficiency_after: Option<f64>, admitted: bool, change_id: Option<ChangeId> }`.
- [ ] `fn propose_lens(anchor: &AnchorId, vault: &mut Vault, substrate: &mut AnnealSubstrate, assay: &dyn AssayAttribution, registry: &mut LensRegistry, profiler: &dyn LensProfiler, nmi: &dyn PairNMI, corpus: &[Constellation]) -> Result<ProposalOutcome, CalyxError>` — full pipeline:
  1. `localize(assay, anchor, &vault.panel)` → `DeficitMap`; if `!has_deficit` return `ProposalOutcome { admitted: false }` immediately.
  2. `synthesize(deficit, corpus)` → `CandidateLens`.
  3. `gate(candidate, &vault.panel, profiler, nmi, corpus)` → if `Rejected`, return `ProposalOutcome { admitted: false, gate_outcome }`.
  4. `substrate.propose_change(vault, HotAddAction { candidate })` → `ChangeOutcome`; if `Revert`, return `ProposalOutcome { admitted: false }`.
  5. Re-measure: `assay.panel_sufficiency(anchor)` → `sufficiency_after`; if `sufficiency_after <= sufficiency_before`, call `substrate.rollback_explicit(change_id)`; return `ProposalOutcome { admitted: false }`.
  6. Return `ProposalOutcome { admitted: true, sufficiency_before, sufficiency_after, change_id }`.
- [ ] `HotAddAction` implements `AnnealAction` wrapping the Registry hot-add (PH20); shadow test = re-measure sufficiency on a held-out set.
- [ ] Ledger entry written at every terminal state (admitted, rejected, rolled-back).
- [ ] All steps within background budget; budget exhausted → abort + `CALYX_ANNEAL_BUDGET_EXHAUSTED`.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: full pipeline with a candidate that clears the gate AND improves sufficiency → `admitted=true`; `sufficiency_after > sufficiency_before`.
- [ ] unit: candidate clears gate but hot-add is reverted by substrate (tripwire) → `admitted=false`; Ledger has `Revert` entry; panel unchanged.
- [ ] unit: `sufficiency_after <= sufficiency_before` after hot-add → rollback; `admitted=false`; panel back to original.
- [ ] edge: `has_deficit=false` (panel already sufficient) → early return `admitted=false` without any synthesis; proposal loop does not run in a well-covered vault.
- [ ] fail-closed: Registry hot-add fails (`CALYX_REGISTRY_HOT_ADD_FAIL`) → `ProposalOutcome { admitted: false }`; no partial hot-add left in the panel.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** vault panel lens list, `I(panel;anchor)` before/after, Ledger `LensAdmitted` entry.
- **Readback:** `calyx anneal lens-proposal-log --last 3`; `calyx assay sufficiency --anchor <id>` before and after.
- **Prove:** on a corpus with a known sufficiency gap: run `propose_lens`; `lens-proposal-log` shows `LensAdmitted` with `sufficiency_before=X, sufficiency_after=Y, Y>X`; `calyx assay sufficiency` confirms the improved value.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH47 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
