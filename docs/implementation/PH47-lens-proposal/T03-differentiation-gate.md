# PH47 · T03 — Differentiation gate (≥0.05 bits, ≤0.6 corr)

| Field | Value |
|---|---|
| **Phase** | PH47 — Lens Proposal (Sufficiency Deficit) |
| **Stage** | S10 — Anneal + Intelligence Objective J |
| **Crate** | `calyx-anneal` |
| **Files** | `crates/calyx-anneal/src/propose/differentiation_gate.rs` (≤500) |
| **Depends on** | T02 (CandidateLens output), PH21 (capability cards — profile called here), PH29 (differentiation contract thresholds) |
| **Axioms** | A7 |
| **PRD** | `dbprdplans/12 §5`, `dbprdplans/07 §4` |

## Goal

Implement the differentiation gate that a `CandidateLens` must clear before
being hot-added to a panel: (1) profile the candidate via a Registry capability
card to get its `bits_per_anchor` on the corpus; (2) compute its `NMI` with
each existing lens in the panel; (3) admit iff `bits_per_anchor ≥ 0.05` AND
`max_corr ≤ 0.6` (using partitioned NMI from PH28 as the correlation proxy).
Rejection returns a structured reason for the Ledger entry.

## Build (checklist of concrete, code-level steps)

- [ ] `enum GateOutcome { Admitted { bits: f64, max_corr: f64 }, Rejected { reason: RejectReason } }` where `RejectReason { InsufficientBits { bits: f64, threshold: f64 }, TooCorrelated { corr: f64, offending_lens: LensId, threshold: f64 }, ProfileTimeout }`.
- [ ] `trait LensProfiler { fn profile(candidate: &CandidateLens, corpus_sample: &[Constellation]) -> Result<CapabilityCard, CalyxError>; }` — bridged from `calyx-registry` (PH21).
- [ ] `trait PairNMI { fn nmi(lens_a: &LensId, lens_b_embeddings: &[Vec<f32>]) -> Result<f64, CalyxError>; }` — bridged from `calyx-assay` (PH28).
- [ ] `fn gate(candidate: &CandidateLens, panel: &[LensId], profiler: &dyn LensProfiler, nmi: &dyn PairNMI, corpus: &[Constellation]) -> GateOutcome` — (a) call `profiler.profile` with timeout `30s`; on timeout → `Rejected { ProfileTimeout }`; (b) check `card.bits_per_anchor >= 0.05`; on fail → `Rejected { InsufficientBits }`; (c) for each lens in panel compute `nmi(candidate, lens_embeddings)`; if any > `0.6` → `Rejected { TooCorrelated }`; (d) all pass → `Admitted`.
- [ ] Thresholds `0.05` and `0.6` are the no-compress values from PRD A7/`07 §4`; configurable only via explicit `set_objective_weights`, not arbitrary overrides.
- [ ] Profile timeout clock-injected.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: `bits=0.04` → `Rejected { InsufficientBits { bits: 0.04, threshold: 0.05 } }`.
- [ ] unit: `bits=0.10`, `max_corr=0.65` → `Rejected { TooCorrelated { corr: 0.65, threshold: 0.6 } }`.
- [ ] unit: `bits=0.10`, `max_corr=0.55` → `Admitted { bits: 0.10, max_corr: 0.55 }`.
- [ ] proptest: for any `(bits, corr)` pair, `gate` returns `Admitted` iff `bits >= 0.05 AND corr <= 0.6`.
- [ ] edge: empty panel (no existing lenses) → corr check passes trivially (`max_corr=0.0`); `bits=0.05` exactly → `Admitted` (boundary is inclusive); profile returns `NaN` bits → `CALYX_ASSAY_INVALID_METRIC`.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `GateOutcome` returned by `gate` + Ledger `LensAdmitted`/`LensRejected` entries.
- **Readback:** `calyx anneal lens-proposal-log --last 5` — prints outcomes with bits, max_corr, reason.
- **Prove:** prepare two candidates: one with `bits=0.03` (below threshold) and one with `bits=0.12, corr=0.45` (above threshold). Run `gate` for both; `lens-proposal-log` shows `Rejected` for the first and `Admitted` for the second with exact metric values.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH47 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
