# Stage 5 — Loom + Assay (DDA & Bits) (PH27–PH30)

> **STATUS: ✅ DONE (FSV-signed-off, commit `0ada102`).** `calyx-loom` implements
> cross-term kinds, eager agreement, lazy Delta/Interaction/Concat with LRU,
> materialization policy, agreement graph, blind-spot detection, signal
> provenance tags, and honest abundance reporting. `calyx-assay` implements
> KSG-style MI, deterministic projection, bootstrap CI, partitioned NMI,
> logistic-probe MI, AssayGate lens/pair signal, differentiation contract,
> stratified bits, n_eff stable rank, sufficiency, attribution, and assay
> cache/store provenance. FSV root:
> `/home/croyse/calyx/data/fsv-stage5-loom-assay-20260608-final`; readback
> hashes are recorded in GitHub #23 and #189. Next active stage is Lodestar
> (`16_STAGE6_LODESTAR.md`).
> Post-sweep hardening #285 makes Loom cross-term math fail closed and adds an
> explicit nonnegative `agreement_weight` beside raw cosine agreement for
> Lodestar graph handoff.

Loom weaves cross-terms (associations between associations) and the agreement
graph; Assay measures the bits each lens/pair carries about real outcomes and
enforces the differentiation contract. Lands in `calyx-loom` + `calyx-assay`.
**Living-system role:** cognition (Loom) + differentiation/self-model (Assay).

> Honesty is load-bearing here: report `C(N,2)` only as an upper bound capped
> by the DPI ceiling and `n_eff` (A8); never sell cross-terms as free info.

---

## PH27 — Agreement graph + cross-terms (lazy)
- **Objective.** Per-constellation agreement vector + lazy cross-terms; storage
  `O(n·n_eff)` not `O(n·N²)`.
- **Deps.** PH24.
- **Deliverables.** `cross_term.rs` (Agreement eager scalar; Delta/Interaction/
  Concat lazy or Assay-gated eager), `agreement_graph.rs` (vault-wide), blind-
  spot detector.
- **Key tasks.** agreement = batched normalized matmul (Forge); lazy xterm =
  one matmul on demand + LRU cache; materialize only Assay-gated pairs.
- **Post-sweep note.** Cross-term APIs now return `Result` with
  `CALYX_LOOM_ZERO_NORM_VECTOR`, `CALYX_LOOM_DIM_MISMATCH`,
  `CALYX_LOOM_NON_FINITE_VECTOR`, and `CALYX_LOOM_SLOT_MISSING`; agreement graph
  edges include raw cosine plus `agreement_weight = clamp(raw, 0, 1)` (#285).
- **FSV gate.** agreement scalars eager + correct; a lazy pair computes on demand
  and matches; **materialized count ≪ C(N,2)** (read xterm CF size); blind-spot
  fires on a planted cross-lens disagreement.
- **Axioms/PRD.** A8, A9, `06 §3/§4/§5`.

## PH28 — KSG MI + partitioned NMI
- **Objective.** Mutual-information estimators correct on small grounded
  samples, with CI + sample count, fail-closed below quorum.
- **Deps.** PH27, PH13 (knn).
- **Deliverables.** `ksg.rs` (k-NN MI via the ANN graph), `nmi.rs` (partitioned
  histogram, streaming), bootstrap CI, random-projection pre-step for high-d.
- **Key tasks.** KSG continuous↔discrete; quorum n≥50 → else
  `CALYX_ASSAY_INSUFFICIENT_SAMPLES`; CI on every estimate.
- **FSV gate.** MI on a **planted-signal synthetic** is within CI of the known
  value; n<50 fails closed (no noisy point estimate).
- **Axioms/PRD.** A2 (grounded only), A16, `07 §2`.

## PH29 — Differentiation contract + n_eff
- **Objective.** Gate lens admission: ≥0.05 bits about a real outcome, ≤0.6
  pairwise correlation; compute effective rank.
- **Deps.** PH28.
- **Deliverables.** `contract.rs` (`admit_lens` → Admit|Reject{reason}),
  `n_eff.rs` (stable rank of the redundancy graph), stratified bits + recurrence
  anchor (refines A7, `26 §9`).
- **Key tasks.** `CALYX_ASSAY_LOW_SIGNAL` / `_REDUNDANT`; per-stratum bits so a
  rare-class sole carrier isn't lost; **no raw-frequency multiplier on bits**.
- **FSV gate.** a **planted-redundant** lens (corr>0.6) is REJECTED; a <0.05-bit
  lens is REJECTED; `n_eff` matches the known rank of a planted panel (read the
  stored decision rows).
- **Axioms/PRD.** A7, A9, `07 §3/§3c`, `26 §9`.

## PH30 — Panel sufficiency + attribution + reports
- **Objective.** `I(panel;anchor)` vs `H(anchor)` (the substrate-sufficiency
  test) + per-sensor decomposition + the honest dashboards.
- **Deps.** PH29.
- **Deliverables.** `sufficiency.rs`, `attribution.rs` (per-slot marginal bits,
  sole-carrier flag), `abundance_report` (N, C(N,2), materialized, n_eff, DPI
  ceiling), `bits_report`.
- **Key tasks.** DPI ceiling exposed; deficit localized to slots; sufficiency
  routes to Anneal lens-proposal (Stage 10).
- **FSV gate.** `abundance_report` prints the four honest numbers; a known-
  insufficient panel (`I≪H`) is flagged with the per-slot deficit (read it);
  trusted bits only when grounded (else `provisional`).
- **Axioms/PRD.** A2, A8, `07 §4/§5`, `06 §1` (meaning compression).

---

## Stage 5 exit — ✅ achieved
Calyx knows, in bits, what every lens is worth and whether the panel can even
answer the question, with the DPI ceiling reported and the differentiation
contract gated before merge — PRD `DDA_BITS`. Feeds Lodestar (kernel) and Anneal
(the objective `J`).
