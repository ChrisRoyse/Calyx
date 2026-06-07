# PH48 · T03 — Gradient priority queue (`ΔJ/cost`, `next_best_action`)

| Field | Value |
|---|---|
| **Phase** | PH48 — J Objective + Growth Curve + Intelligence Report |
| **Stage** | S10 — Anneal + Intelligence Objective J |
| **Crate** | `calyx-anneal` |
| **Files** | `crates/calyx-anneal/src/j/gradient.rs` (≤500) |
| **Depends on** | T01 (JValue), T02 (GoodhartChecker gates promotions) |
| **Axioms** | A32 |
| **PRD** | `dbprdplans/27 §3` |

## Goal

Implement `IntelligenceGradient`: the priority queue of candidate actions ranked
by estimated `ΔJ / cost`. For each candidate action type (propose lens, label
anchor, prune redundant lens, recalibrate/heal, recompute kernel, materialize
cross-term, retune math), estimate the expected `ΔJ` gain and the cost (compute
budget units), and expose `next_best_action() -> CandidateAction` — the top-
priority action for the current vault state. Greedy gradient ascent on grounded
intelligence, cost-aware, from `27 §3`.

## Build (checklist of concrete, code-level steps)

- [ ] `enum CandidateAction { ProposeLens { anchor: AnchorId, estimated_dj: f64 }, LabelAnchor { anchor: AnchorId, estimated_dj: f64 }, PruneRedundantLens { lens_id: LensId, estimated_dj: f64 }, RecalibrateHeal { component: ComponentKind, estimated_dj: f64 }, RecomputeKernel { scope: ScopeId, estimated_dj: f64 }, MaterializeCrossTerm { pair: (LensId, LensId), estimated_dj: f64 }, RetuneMath { scope: TuneScopeKind, estimated_dj: f64 } }`.
- [ ] `struct GradientEntry { action: CandidateAction, dj_per_cost: f64, cost_budget_units: u64 }` — priority is `dj_per_cost`.
- [ ] `struct IntelligenceGradient { queue: BinaryHeap<GradientEntry>, current_j: JValue, clock: Arc<dyn Clock> }`.
- [ ] `fn estimate_dj(action: &CandidateAction, vault: &Vault, assay: &dyn JMetricSources) -> f64` — heuristic per action type:
  - `ProposeLens`: `I(panel∪candidate; oracle) − I(panel; oracle)` from Assay.
  - `LabelAnchor`: expected info gain from the grounding gap (PH33 `grounding_gaps`).
  - `PruneRedundantLens`: `Δn_eff` × `w2` from the `JWeights`.
  - `RecomputeKernel`: `(target_recall − current_kernel_recall) × w4`.
  - `MaterializeCrossTerm`: `Δbits` for the pair × `w1`.
  - `RetuneMath`: `Δp99_fraction × w_latency` (freed compute capacity proxy).
  - `RecalibrateHeal`: `Δoracle_accuracy × w5`.
- [ ] `fn refresh(vault: &Vault, assay: &dyn JMetricSources)` — recomputes all `GradientEntry` estimates; re-heaps; called periodically.
- [ ] `fn next_best_action(&self) -> Option<&CandidateAction>` — peeks at the max `dj_per_cost` entry.
- [ ] `fn set_objective_weights(weights: JWeights)` — updates the weights used in `estimate_dj`; stored in vault config; persisted.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: three actions with `dj_per_cost` `[0.5, 2.0, 1.0]` → `next_best_action` returns the `2.0` entry.
- [ ] unit: `estimate_dj(ProposeLens)` with known `I_before=0.3, I_after=0.8` → `estimated_dj=0.5`.
- [ ] proptest: for any set of actions, `next_best_action` returns the one with highest `dj_per_cost` (heap invariant).
- [ ] edge: empty queue → `next_best_action` returns `None`; all actions with `dj_per_cost=0.0` → any one returned (tie-breaking by insertion order); `cost_budget_units=0` → action has infinite priority (free improvement); filtered out if cost > current budget.
- [ ] fail-closed: `estimate_dj` returns `NaN` → treated as `0.0` (no gain); error logged; action excluded from queue.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `IntelligenceGradient` queue state + `next_best_action` output.
- **Readback:** `calyx anneal intelligence-report` — prints `gradient: [(action, ΔJ/cost)]` top 5.
- **Prove:** on a vault with a known sufficiency deficit: `intelligence-report` shows `ProposeLens` as the top gradient action with a non-zero estimated `ΔJ/cost`; after the lens is proposed and admitted, the `ProposeLens` entry is removed and the next action is promoted.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH48 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
