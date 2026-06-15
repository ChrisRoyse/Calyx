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
> hashes are recorded in GitHub #23 and #189. Later stage FSV has consumed
> Stage 5 successfully; current active frontier is tracked in `03_PHASE_MAP.md`.
> Post-sweep hardening #285 makes Loom cross-term math fail closed and adds an
> explicit nonnegative `agreement_weight` beside raw cosine agreement for
> Lodestar graph handoff.
> Post-sweep hardening #287 scopes Assay cache/store rows by vault and anchor
> so Stage 6 cannot consume signal bits measured for a different vault or
> outcome axis.
> Post-sweep hardening #291 makes that scope mandatory for persisted Assay CF
> rows and makes KSG/logistic estimators reject ragged or non-finite sample
> matrices before computing bits.
> Post-sweep hardening #294 gates Assay trust tags on grounded Anchor evidence:
> no-anchor estimators/report helpers are `provisional`; anchor-aware paths emit
> `trusted` only for non-empty finite-confidence anchors. FSV root:
> `/home/croyse/calyx/data/fsv-issue294-assay-grounded-trust-20260608`.
> Post-sweep hardening #309 makes `AssayGate.min_samples` authoritative,
> keeps Delta/Concat lazy while only qualifying Interaction is extra eager, and
> reports `meaning_compression_yield` as materialized signals per input.
> FSV root: `/home/croyse/calyx/data/fsv-issue309-stage5-gates-abundance-20260608`.
> Post-sweep hardening #313 removes the fake GPU agreement alias: the default
> build returns `CALYX_LOOM_FORGE_UNAVAILABLE` for `agreement_batch_gpu`, while
> the `calyx-loom/cuda` feature calls the real Forge CUDA backend. FSV root:
> `/home/croyse/calyx/data/fsv-issue313-loom-gpu-agreement-20260608`.
> Post-sweep hardening #317 makes partitioned NMI fail closed before binning for
> mismatched, empty, n<50, and NaN/Inf scalar streams, while accepting n=50
> exactly. FSV root:
> `/home/croyse/calyx/data/fsv-issue317-nmi-fail-closed-20260608`.
> Post-sweep hardening #318 wires deterministic seeded bootstrap CI into the
> public KSG estimator, logistic-probe path, AssayGate lens signal, PairGain
> estimate, and persisted AssayStore rows. FSV root:
> `/home/croyse/calyx/data/fsv-issue318-bootstrap-ci-20260608`.
> Post-sweep hardening #319 adds an Aster-backed Assay materialization gate for
> Loom: slot/anchor rows are read from AsterVault, PairGain feeds
> `plan_cross_terms`, and eager plan entries are physically materialized into
> the Loom xterm CF. FSV root:
> `/home/croyse/calyx/data/fsv-issue319-aster-materialization-gate-20260608`.
> Contract-honesty hardening #340 adds real UCI Iris labeled-classification FSV
> with pinned dataset hash, derived anchor entropy, Assay CF readback, and xterm
> CF readback; makes Assay `project_gpu` fail loud instead of aliasing CPU
> projection; adds finite guards to `admit_lens`; makes Aster-backed
> materialization planning return errors by default; and documents that
> abundance/sufficiency/blind-spot APIs are helper/report surfaces until PH62
> CLI/MCP vault-backed product entry points land (PH62 T05/T07/T08 and PH63
> T06/T08).

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
- **Key tasks.** agreement = normalized cosine on the CPU path by default;
  explicit `agreement_batch_gpu` fails closed unless `calyx-loom/cuda` is
  enabled, where it dispatches through Forge CUDA; lazy xterm = one compute on
  demand + LRU cache; materialize Agreement eagerly, keep Delta/Concat lazy, and
  materialize Interaction only for Assay-gated pairs.
- **Post-sweep note.** Cross-term APIs now return `Result` with
  `CALYX_LOOM_ZERO_NORM_VECTOR`, `CALYX_LOOM_DIM_MISMATCH`,
  `CALYX_LOOM_NON_FINITE_VECTOR`, and `CALYX_LOOM_SLOT_MISSING`; agreement graph
  edges include raw cosine plus `agreement_weight = clamp(raw, 0, 1)` (#285).
- **Post-sweep note.** Materialization is per kind: Agreement is eager, Delta
  and Concat remain lazy, and only Interaction becomes eager when the Assay pair
  gain clears 0.05 bits (#309).
- **Post-sweep note.** The Aster-backed materialization gate no longer hides
  missing slot/anchor errors behind a default `0.0` pair gain. Use
  `AsterAssayMaterializationGate::materialization_plan` for fail-closed
  planning; `materialization_plan_fail_safe_lazy` is the explicit opt-in
  fallback for callers that want to keep Agreement eager, park Interaction lazy,
  and inspect `last_error` (#340).
- **Post-sweep note.** GPU agreement is no longer a CPU alias. Default builds
  return `CALYX_LOOM_FORGE_UNAVAILABLE`; `calyx-loom/cuda` compiles and executes
  the Forge CUDA path on aiwonder (#313).
- **FSV gate.** agreement scalars eager + correct; a lazy pair computes on demand
  and matches; persisted xterm rows show Agreement-only equals `C(N,2)` per
  input while Delta/Concat stay lazy and Interaction is extra eager only when
  Assay-gated (read xterm CF kind counts); blind-spot fires on a planted
  cross-lens disagreement.
- **Axioms/PRD.** A8, A9, `06 §3/§4/§5`.

## PH28 — KSG MI + partitioned NMI
- **Objective.** Mutual-information estimators correct on small grounded
  samples, with CI + sample count, fail-closed below quorum.
- **Deps.** PH27, PH13 (knn).
- **Deliverables.** `ksg.rs` (k-NN MI via the ANN graph), `nmi.rs` (partitioned
  histogram, streaming), bootstrap CI, random-projection pre-step for high-d.
- **Key tasks.** KSG continuous↔discrete; quorum n≥50 → else
  `CALYX_ASSAY_INSUFFICIENT_SAMPLES`; CI on every estimate.
- **Post-sweep note.** Persisted Assay rows now include vault and anchor scope
  in the cache key and in the physical Aster Assay CF key (#287); unscoped
  rows fail closed before persistence/load, and estimator sample matrices must
  be finite and rectangular (#291). KSG/logistic estimates are `provisional`
  without an Anchor; the `_with_anchor` variants emit `trusted` only for
  grounded Anchor evidence (#294). `AssayGate.min_samples` now controls the
  logistic-probe quorum used by lens and pair signal calls (#309). Partitioned
  NMI shares the n>=50/malformed-input fail-closed contract (#317). KSG,
  logistic-probe, AssayGate lens signal, and PairGain estimates now attach
  seeded bootstrap CI through the public paths before persistence (#318). The
  Aster-backed Assay materialization gate now feeds real grounded PairGain into
  Loom planning and xterm CF materialization (#319). Assay `project_gpu` is no
  longer a CPU alias; it returns `CALYX_FORGE_DEVICE_UNAVAILABLE` until a real
  Forge-backed projection path exists (#340). Real-data FSV now runs on the UCI
  Iris labeled classification dataset and writes pinned dataset hash, row count,
  derived anchor entropy, Assay CF readback, and xterm CF readback (#340).
- **FSV gate.** MI on a **planted-signal synthetic** is within CI of the known
  value; n<50 fails closed (no noisy point estimate).
- **Axioms/PRD.** A2 (grounded only), A16, `07 §2`.

## PH29 — Differentiation contract + n_eff
- **Objective.** Gate lens admission: ≥0.05 bits about a real outcome, ≤0.6
  pairwise correlation; compute effective rank.
- **Deps.** PH28.
- **Deliverables.** `contract.rs` (`admit_lens` → Admit|Reject{reason}),
  `n_eff.rs` (stable rank of the redundancy graph), stratified bits +
  no-frequency-multiplier invariant (refines A7, `26 §9`); typed recurrence
  rate/CI semantics are PH42 T01/T06/T07 work, with Bayesian recurrence CIs in
  PH52 T04 (#340).
- **Key tasks.** `CALYX_ASSAY_LOW_SIGNAL` / `_REDUNDANT`; per-stratum bits so a
  rare-class sole carrier isn't lost; **no raw-frequency multiplier on bits**.
- **Post-sweep note.** `admit_lens` rejects NaN/Inf `signal_bits` and
  `max_pairwise_corr` before threshold comparisons (#340).
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
  emits structured deficits through `SufficiencyDeficitSink`; Anneal consumes
  that interface in PH47 T01-T05.
- **Post-sweep note.** Sufficiency and bit reports remain anchor-explicit; the
  backing AssayStore now prevents same-panel/shard/subject collisions across
  vaults or anchors (#287), and persisted rows without vault scope are rejected
  with `CALYX_VAULT_ACCESS_DENIED` (#291). No-anchor report helpers downgrade
  requested `trusted` tags to `provisional`; use the anchor-aware helpers to
  report grounded trusted bits (#294). `meaning_compression_yield` is now the
  materialized cross-term count per input, with NaN for zero constellations
  (#309).
- **Post-sweep note.** Current abundance, sufficiency, attribution, and
  blind-spot surfaces are library helpers/reports used by tests and later
  engines. Vault-backed user-facing commands/tools are deferred to PH62/PH63
  (PH62 T05/T07/T08 and PH63 T06/T08). This Stage 5 contract is the measured
  report data and CF persistence, not a complete CLI/MCP product surface (#340).
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
