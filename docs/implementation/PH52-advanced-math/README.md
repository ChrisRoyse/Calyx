# PH52 — Advanced Math (Spectral / Energy / Transfer-Entropy / TC / Bayesian)

**Stage:** S11 — Oracle & AGI Layer  ·  **Crate:** `calyx-assay` + `calyx-oracle`  ·
**PRD roadmap:** `26 §2–§11`  ·  **Axioms:** A30, A2, A16

## Objective

Implement the **Build**-tier math from `dbprdplans/26` that the architecture makes
available but the core engines don't yet use. Each item reuses existing Forge primitives
and Anneal autotuning — no new external dependencies. Each new number is proven against
a **planted synthetic** (planted period, planted causal A→B, planted rare-class carrier,
planted community) by reading the computed value — not a harness. Every result carries a
CI; fail-closed below quorum. Complements (never replaces) the grounded MFVS kernel.

**Six capabilities delivered:**

| Capability | PRD | Home crate |
|---|---|---|
| Spectral centrality + GFT (graph Fourier transform) | `26 §2` | `calyx-mincut` / Forge |
| Energy pattern-completion (standalone math layer) | `26 §3` | `calyx-oracle` (via `complete.rs`) |
| Periodicity (Lomb-Scargle + autocorrelation) on recurrence streams | `26 §4` | `calyx-assay` |
| Inter-event hazard ("overdue" anomaly) + CUSUM rate change-point | `26 §4` | `calyx-assay` |
| Transfer entropy `T(A→B) = I(B_future; A_past | B_past)` on recurrence streams | `26 §4` | `calyx-assay` |
| Total correlation `TC(Φ)` / `n_eff` | `26 §5` | `calyx-assay` |
| Bayesian posteriors (Gamma-Poisson rate, Beta-Bernoulli consistency) | `26 §6` | `calyx-assay` |
| Grounded label propagation (Laplacian heat diffusion) | `26 §11.2` | `calyx-lodestar` / `calyx-mincut` |

Energy pattern-completion is the `complete()` function from PH51 — PH52 ensures the
mathematical layer (energy.rs) is tested against the planted synthetic directly.

> Honesty is the feature: spectral centrality is a *complement* to the grounded MFVS
> kernel — centrality proposes kernel candidates that grounding confirms (A2). Every
> number carries CI; provisional on short series.

**Current state:** `calyx-oracle` has `energy.rs` from PH51. `calyx-assay` has KSG MI
from PH28; transfer entropy and TC are new. `calyx-mincut` has the graph from PH31;
eigensolver is new. `calyx-lodestar` has the kernel from PH32; label propagation is new.

## Dependencies

- **Phases:** PH51 (`complete` + energy; energy pattern-completion is PH51's T01),
  PH28 (KSG MI machinery — transfer entropy reuses the estimator),
  PH31 (mincut sparse graph — spectral eigensolver operates on it),
  PH32 (directed MFVS kernel — spectral centrality complements it),
  PH42 (grounded recurrence streams — transfer entropy and Bayesian posteriors operate on them),
  PH46 (Anneal autotune — β for energy, lag for transfer entropy)
- **Provides for:** PH70 (intelligence validation FSV — advanced math numbers are part of the
  full intelligence validation on real corpora)

## Current state (build off what exists)

Greenfield for all six capabilities. Existing infrastructure: Forge SIMD/CUDA (PH12/PH13),
KSG estimator (PH28), sparse graph (PH31), Anneal config (PH46). New files in this phase
span three crates (`calyx-assay`, `calyx-mincut`, `calyx-oracle`).

## Deliverables (file plan, each ≤500 lines)

| File | Crate | Responsibility |
|---|---|---|
| `src/spectral.rs` | `calyx-mincut` | Eigenvector centrality + Lanczos eigensolver; GFT project/reconstruct; spectral gap |
| `src/periodicity.rs` | `calyx-assay` | Floating-mean Lomb-Scargle periodogram, event-count binning, slotted autocorrelation, seeded permutation FAP |
| `src/recurrence_hazard.rs` | `calyx-assay` | Gamma-renewal inter-event hazard → overdue flag; Page CUSUM on the gap series → rate change-point |
| `src/special_fn.rs` | `calyx-assay` | Shared deterministic special functions: regularised incomplete gamma + Lanczos `ln Γ` |
| `tests/recurrence_hazard_fsv.rs` | `calyx-assay` | Planted-synthetic + vault-readback FSV for hazard/overdue and CUSUM change-point |
| `src/transfer_entropy.rs` | `calyx-assay` | `T(A→B) = I(B_future; A_past | B_past)` on recurrence streams; reuses KSG |
| `src/total_correlation.rs` | `calyx-assay` | `TC(Φ) = ΣH(slot_k) − H(Φ)`; interaction information; `n_eff` from TC |
| `src/bayesian.rs` | `calyx-assay` | Gamma-Poisson rate posterior; Beta-Bernoulli consistency posterior; credible intervals |
| `src/label_propagation.rs` | `calyx-lodestar` | Laplacian heat diffusion from kernel anchors; propagated grounding confidence |
| `tests/advanced_math_fsv.rs` | `calyx-assay` | Planted-synthetic FSV tests for all five new numbers |

## Tasks (atomic — all must pass for the phase to be DONE)

| Card | Title | Depends |
|---|---|---|
| T01 | Spectral centrality + GFT (Lanczos + Forge eigensolve) | PH31 graph |
| T01b | Lomb-Scargle + autocorrelation periodicity build card | PH42 recurrence streams |
| T01c | Inter-event hazard (overdue) + CUSUM rate change-point | PH42 recurrence streams |
| T02 | Transfer entropy on recurrence streams (reuse KSG) | PH28, PH42 |
| T03 | Total correlation `n_eff` (TC + interaction information) | PH28 |
| T04 | Bayesian posteriors: Gamma-Poisson + Beta-Bernoulli | PH42 |
| T05 | Grounded label propagation (Laplacian heat diffusion) | PH32, PH31 |
| T06 | FSV: all five numbers proven against planted synthetics | T01–T05 |

## FSV exit gate (the phase is DONE only when this is byte-proven on aiwonder)

Each new number proven against a planted synthetic (read computed vs known):
1. **Spectral:** planted community in synthetic graph -> spectral centrality ranks community hub correctly.
1b. **Periodicity:** planted 7.0-unit recurrence stream -> Lomb-Scargle dominant period recovers within +/-5%, false-alarm probability is significant, and autocorrelation independently reports the fundamental lag.
1c. **Hazard / CUSUM:** planted 100s cadence that stops -> overdue hazard fires at the expected elapsed (survival ≤ α) and not before; planted 5× rate shift -> CUSUM localises the change-point at the planted occurrence index with the correct direction; a steady cadence fires no false alarm.
2. **Transfer entropy:** planted causal A→B (A always precedes B) → `T(A→B) > T(B→A)` + CI
3. **Total correlation / `n_eff`:** `n_eff` from TC < N (some redundancy) for a known-redundant
   panel; interaction information positive for a known-synergistic triple
4. **Bayesian posteriors:** Gamma-Poisson credible interval contains true rate after n=5 events;
   Beta-Bernoulli CI contains true pass-rate after n=10 trials
5. **Label propagation:** kernel anchors propagate grounding to 2-hop neighbors with confidence
   decaying monotonically with graph distance; planted rare-class carrier recovers its label

## Risks / landmines

- **Eigensolver numerical stability:** Lanczos on sparse near-singular graphs can produce
  non-orthogonal eigenvectors; orthogonalize every `k` steps (standard practice); test with
  known planted eigenvalues ± 1e-3.
- **Transfer entropy lag selection:** the "right" lag is domain-dependent; expose as Anneal-
  tunable; provide a default sweep `[1, 2, 4, 8]` and report the max-TE lag.
- **Total correlation dimensionality:** KSG TC in high-d is noisy; enforce quorum ≥ `50 × N`
  samples where N = number of lenses (fail-closed below quorum, A16).
- **Label propagation convergence:** Laplacian heat diffusion over a disconnected graph
  never converges for isolated components; detect and mark as `provisional` with error code.
- **Spectral centrality vs kernel:** never replace the grounded MFVS kernel with centrality;
  the kernel is outcome-anchored (A2); centrality is structure-only. Add a comment/assert
  in `spectral.rs` documenting this boundary.
