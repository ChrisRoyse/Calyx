# Stage 11 — Oracle & AGI Layer (PH49–PH52)

The AGI substrate of the Royse corpus, strictly from it: predict the grounded
consequences of actions (the world model), measure per-domain whether super-
intelligence is reached, walk meaning both ways (Q↔A), and unify predict/abduce/
impute. Lands in `calyx-oracle`. **Living-system role:** foresight / agency.

> Honesty is the feature: the same machine that predicts also **falsifies its
> own ability to predict, cheaply, before training** (the ME-JEPA discipline).

---

## PH49 — Consequence prediction + sufficiency gate
- **Objective.** `oracle_predict(action, domain)` → outcome + calibrated
  confidence (capped at oracle self-consistency) + consequences + the sufficiency
  bound + provenance + guard.
- **Deps.** PH48, PH42 (grounded recurrence), PH30 (sufficiency).
- **Deliverables.** `predict.rs` (JEPA step: `(panel_t, action)→panel_{t+1}/
  outcome`), `expand`/`select` (butterfly tree, hop-attenuated), honesty gate.
- **Key tasks.** confidence capped at `oracle_self_consistency` (measured from
  recurrence, `07 §3b`); **refuse** when `I(panel;oracle)<H(outcome)` →
  `sufficient:false` + per-sensor deficit (`CALYX_ORACLE_INSUFFICIENT`);
  evidence is grounded recurrence (empirical rate/cadence).
- **FSV gate.** on a real deterministic-oracle domain (**SWE-bench Lite** on
  aiwonder): predict Pass/Fail; on a form-only panel measure the **≈0.46-bit
  deficit** → sufficiency-refusal fires (read it); confidence never exceeds the
  ceiling.
- **Axioms/PRD.** A20, A2, A8, `21 §1/§2`.

## PH50 — Super-intelligence predicate + reverse_query
- **Objective.** The falsifiable 6-tier predicate per domain + epistemic
  symmetry (answer→question/cause).
- **Deps.** PH49.
- **Deliverables.** `super_intelligence(domain)` (oracle_clean ∧ panel_sufficient
  ∧ kernel_exists ∧ calibrated ∧ goodhart_defended ∧ mistake_closed →
  {tiers, failing_tier, cheapest_fix}), `reverse_query(answer)` (asymmetric
  back-edges + kernel-toward-antecedents).
- **Key tasks.** each tier measured against held-out oracle outcomes (Goodhart-
  defended); reverse traversal over **grounded** edges only (else `provisional`).
- **FSV gate.** the predicate reports the failing tier + cheapest fix on a real
  domain; `reverse_query` on a known cause **recovers it**; ungrounded reverse →
  labeled provisional (read verdicts).
- **Axioms/PRD.** A20, A23, `21 §3/§5`, `17 §7.5` (Goodhart/hallucination).

## PH51 — complete() unified primitive (predict=abduce=impute)
- **Objective.** One energy-descent primitive; forward/reverse/lateral are
  clamp-direction choices.
- **Deps.** PH50, PH37 (energy/Gτ region).
- **Deliverables.** `complete(cx, clamp, free)` → filled cx + confidence
  (clamp present/free future = predict; clamp outcome/free cause = abduce; clamp
  some lenses/free rest = impute); energy `E(x)=−logΣ exp(β·sim)`.
- **Key tasks.** few descent steps updating absent slots; β Anneal-tuned; filled
  slots tagged `inferred`/`provisional`, confidence capped, refuse if
  insufficient.
- **FSV gate.** a partial constellation completes to the known full one within
  tolerance on synthetic; completed slots are tagged `inferred`, never confused
  with measured (read flags).
- **Axioms/PRD.** `26 §11.1`, A2, A16, A20.

## PH52 — Advanced math (spectral / energy / transfer-entropy / TC / Bayesian)
- **Objective.** The math the architecture makes available but the core engines
  don't yet use (PRD `26`, **Build** tier).
- **Deps.** PH51, PH28 (MI machinery).
- **Deliverables.** spectral centrality/GFT (`calyx-mincut` + Forge eigensolve),
  energy pattern-completion (Ward+Forge), transfer entropy on recurrence streams
  (Assay+Oracle), total correlation `n_eff` (Assay), Bayesian posteriors (Gamma-
  Poisson rate, Beta-Bernoulli consistency), grounded label propagation
  (Laplacian heat diffusion).
- **Key tasks.** each reuses existing Forge primitives + Anneal autotune; each
  number reported with CI, fail-closed below quorum; complements (never
  replaces) the grounded MFVS kernel.
- **FSV gate.** each new number proven against a **planted synthetic** (planted
  period via Lomb-Scargle, planted causal A→B via transfer entropy, planted
  rare-class carrier via stratified bits, planted community via spectral) — read
  computed vs known.
- **Axioms/PRD.** `26 §2–§11`, A30, A2.

---

## Stage 11 exit
Calyx predicts grounded consequences with calibrated, sufficiency-gated
confidence, measures per-domain super-intelligence, walks Q↔A, unifies predict/
abduce/impute as one energy descent, and exploits the spectral/energy/temporal-
causal math — PRD `ORACLE`. The engineering half of the AGI claim, built; the
creativity half (novelty frontier × Oracle) architected.
