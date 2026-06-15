# PH29 — Differentiation contract + n_eff

**Stage:** S5 — Loom + Assay (DDA & Bits)  ·  **Crate:** `calyx-assay`  ·
**PRD roadmap:** P4  ·  **Axioms:** A7, A9

## Objective

Gate lens admission: a lens is admitted iff it carries ≥ **0.05 bits** about a
real outcome AND its maximum pairwise correlation with any existing panel member
is ≤ **0.6**. Lenses failing the first gate return `CALYX_ASSAY_LOW_SIGNAL`;
lenses failing the second return `CALYX_ASSAY_REDUNDANT`. Compute `n_eff` as the
stable rank of the redundancy graph (the effective number of non-redundant
lenses). Implement stratified bits — compute MI per outcome stratum so a
rare-class sole carrier is not lost. Typed recurrence anchor rate/CI semantics
are not complete in PH29; they are tracked to PH42 grounded recurrence wiring.
The differentiation contract is enforced at admission and re-checked by Anneal
as the corpus grows (PH47).

> **Binding honesty rules (from `07 §3c`):**
> - **No raw-frequency multiplier on bits.** Bits stay = MI. Multiplying by raw
>   frequency would reward low-information common detectors — forbidden (A8, DPI).
> - **Stratified bits:** admit a lens if it clears ≥ 0.05 bits on **some**
>   grounded stratum, even if aggregate MI < 0.05, so the rare-class sole carrier
>   is not silently discarded.
> - **Recurrence tracking:** the plain `AnchorKind::Recurrence` enum variant is
>   present, but typed recurrence rate/CI semantics are PH42 work. PH29 must not
>   claim trusted recurrence-rate bits until PH42 FSV proves the series wiring.

## Dependencies

- **Phases:** PH28 (KSG MI estimators, bootstrap CI, quorum guard, AssayGate),
  PH27 (agreement graph, redundancy graph backbone ≥0.6 edges)
- **Provides for:** PH30 (panel sufficiency uses n_eff and contract decisions),
  PH47 (Anneal re-checks the contract), PH41 (dedup recurrence wiring)

## Current state (build off what exists)

`calyx-assay` has the MI estimators from PH28 and the `AssayGate` impl.
The `MaterializationPlan` from PH27 has the stub assay gate — now replaced by
the real contract. The redundancy graph backbone (edges ≥ 0.6 mean agreement)
is available from PH27 T04 `agreement_graph`.

## Deliverables (file plan, each ≤500 lines)

| File | Responsibility |
|---|---|
| `crates/calyx-assay/src/contract.rs` | `admit_lens` function: bits gate ≥ 0.05, corr gate ≤ 0.6; `AdmitResult::Admit | Reject { reason }` |
| `crates/calyx-assay/src/stratified.rs` | Per-stratum bits: `I(lens; outcome)` per outcome class; sole-carrier flag; no-frequency-multiplier invariant |
| `crates/calyx-assay/src/n_eff.rs` | `n_eff` = stable rank of the redundancy graph (spectral approach: ratio of squared sum to sum of squares of agreement eigenvalues); replaces PH27 provisional placeholder |
| `crates/calyx-assay/src/tests.rs` | Planted-synthetic FSV: planted-redundant lens REJECTED, planted-signal within CI, known n_eff |

## Tasks (atomic — all must pass for the phase to be DONE)

| Card | Title | Depends |
|---|---|---|
| T01 | `admit_lens`: bits gate + corr gate + error codes | — |
| T02 | Stratified bits + recurrence tracking note + no-multiplier invariant | T01 |
| T03 | `n_eff` stable rank of redundancy graph | T01 |
| T04 | Planted-synthetic FSV: redundant REJECTED, signal admitted, n_eff correct | T01, T02, T03 |

## FSV exit gate (the phase is DONE only when this is byte-proven on aiwonder)

1. **Planted-redundant lens REJECTED (corr > 0.6):** add a candidate lens whose
   slot vector is `v_existing + small_noise` (corr ≈ 0.95 with an existing slot);
   call `admit_lens`; must return `Reject { reason: Redundant }`:
   ```
   cargo test admit_lens_redundant_rejected -- --nocapture
   ```

2. **< 0.05-bit lens REJECTED:** add a candidate lens with random uncorrelated
   vectors (MI ≈ 0.0 bits with the anchor); must return `Reject { reason: LowSignal }`:
   ```
   cargo test admit_lens_low_signal_rejected -- --nocapture
   ```

3. **`n_eff` matches known rank:** create a planted panel with 5 near-identical
   lenses + 3 genuinely independent lenses (known n_eff ≈ 3.0 ± 0.5); read the
   computed n_eff; confirm it is in [2.5, 4.0]:
   ```
   cargo test n_eff_planted_panel -- --nocapture
   ```

4. Read the stored decision rows from the assay CF on aiwonder:
   ```
   calyx readback --cf assay --decisions --since 0
   ```
   Confirm planted-redundant lens has `status: Rejected(Redundant)` and the
   admitted lens has `status: Admitted { bits: f32 }`.

Evidence attached to PH29 GitHub issue.

## Risks / landmines

- **Corr vs NMI disambiguation:** the ≤0.6 gate uses **linear correlation** as
  the first-pass fast gate; for borderline cases (0.5–0.7 corr), promote to
  `pair_redundancy_nmi` from PH28. Do not use NMI alone (it is slower). Both
  gates must be documented in code; the fast-pass is not a shortcut that skips
  the MI check.
- **Stratified bits edge case:** if a stratum has fewer than 50 samples, it
  cannot be used for the sole-carrier check — log the stratum as
  `CALYX_ASSAY_INSUFFICIENT_SAMPLES` for that stratum and continue with other
  strata. Do not fail the whole admission on a single thin stratum.
- **n_eff at small N:** when N < 4 lenses, stable rank may be noisy; report as
  `NeffEstimate::Computed { value, ci_low, ci_high }` with wide CI; never
  clamp to an integer without CI.
- **No raw-frequency multiplier invariant:** add a `#[test]` that explicitly
  checks: `lens_signal(slot, anchor).bits == ksg_with_ci(slot, anchor).bits`
  (bits are not multiplied by frequency or any other scalar). This is a
  regression guard for the binding rule.
