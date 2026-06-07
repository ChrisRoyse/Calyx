# PH47 — Lens Proposal (Sufficiency Deficit)

**Stage:** S10 — Anneal + Intelligence Objective J  ·  **Crate:** `calyx-anneal`  ·
**PRD roadmap:** `12 §5`, `07 §4`  ·  **Axioms:** A7

## Objective

When Assay reports `I(panel; anchor) ≪ H(anchor)` (the panel cannot predict an
outcome), implement `propose_lens(anchor)`: localize the sufficiency deficit via
per-sensor attribution, synthesize or commission a candidate lens targeting the
missing bits, profile it via a Registry capability card, admit it only if it
clears the differentiation contract (≥0.05 bits, ≤0.6 corr with existing lenses),
hot-add it to the panel, and re-measure sufficiency. The fix is the right sensor,
not more training. Every proposal is reversible + Ledger-logged via the PH43
substrate.

## Dependencies

- **Phases:** PH46 (autotune loops; the tuned Loom materialization plan is needed
  to correctly evaluate a new lens's contribution), PH30 (panel sufficiency +
  attribution reports — the deficit measurement and per-sensor attribution are
  sourced here)
- **Provides for:** PH48 (`J` composite `w1·Σ I(panel;anchor)` + `w3·sufficiency`
  both improve when a qualifying lens is added; `lens_proposal` is a candidate
  action in the `gradient.rs` priority queue)

## Current state (build off what exists)

`calyx-anneal` crate: PH43+PH44+PH45+PH46 complete. No lens-proposal logic
exists. Greenfield. Heritage: ContextGraph `embedder_proposal` /
`instrument_proposal` / `embedder_falsification` — logic absorbed into Calyx,
source copied into `CALYX_HOME`.

**Anneal invariants (binding):**
- A lens is admitted only if it clears the differentiation contract: ≥0.05 bits
  of information gain, ≤0.6 correlation with any existing lens (A7).
- Hot-add: no re-embed of existing constellations (PH20 backfill path).
- Every proposal (admission OR rejection) is Ledger-logged.
- Candidate profiling (capability card) runs before any hot-add.
- No data deleted in the proposal process.

## Deliverables (file plan, each ≤500 lines)

| File | Responsibility |
|---|---|
| `src/propose/deficit_localize.rs` | Deficit localization: parse Assay attribution report → identify which input class / anchor type is underrepresented |
| `src/propose/candidate_synth.rs` | Candidate lens synthesis: algorithmic lens construction or commission-on-corpus spec; `CandidateLens` type |
| `src/propose/differentiation_gate.rs` | Differentiation gate: check ≥0.05 bits gain AND ≤0.6 corr; admit or reject with reason |
| `src/propose/propose_lens.rs` | Top-level `propose_lens(anchor)` orchestrator: calls deficit→synth→gate→hot-add→re-measure |
| `src/propose/admission_record.rs` | Ledger entries for every proposal: `LensAdmitted` / `LensRejected`; re-measure sufficiency diff |

## Tasks (atomic — all must pass for the phase to be DONE)

| Card | Title | Depends |
|---|---|---|
| T01 | Deficit localization (Assay attribution → deficit class) | — |
| T02 | Candidate lens synthesis (`CandidateLens` + commission spec) | T01 |
| T03 | Differentiation gate (≥0.05 bits, ≤0.6 corr) | T02 |
| T04 | `propose_lens` orchestrator (hot-add + re-measure) | T01–T03 |
| T05 | Admission record + integration FSV | T01–T04 |

## FSV exit gate (the phase is DONE only when this is byte-proven on aiwonder)

On a known-insufficient panel (where `I(panel;anchor) ≪ H(anchor)` on a real or
realistic synthetic corpus): call `propose_lens(anchor)` → a candidate that
clears the differentiation contract is admitted and hot-added → re-measure
`I(panel;anchor)` → confirm the value increased (sufficiency rose). Separately:
a candidate that does NOT clear the contract (bits < 0.05 or corr > 0.6) is
rejected → Ledger has `LensRejected` entry with reason.

## Risks / landmines

- **Commission-on-corpus** lens (using TEI or local embedding) takes wall time;
  profile before admit — if it hangs, the proposal loop hangs. Use a timeout
  and `CALYX_REGISTRY_PROFILE_TIMEOUT`.
- **Algorithmic synthesis** (e.g., PCA, time-lag) is always fast; prefer it as
  the first candidate if the deficit is in a known algorithmic class.
- **Corr ≤ 0.6 gate**: correlation must be measured against ALL existing lenses
  in the panel, not just the nearest. Use the Assay partitioned NMI (PH28) as
  the correlation proxy.
- **Re-measure sufficiency** after hot-add must use the same Assay measurement
  path as the deficit detection (same anchor set, same panel definition); a
  different measurement = cherry-picking.
- **No data deleted**: the proposal process never removes existing constellations
  to "make room" for a new lens; hot-add only.
