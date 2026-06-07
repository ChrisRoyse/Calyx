# PH45 · T03 — OnlineHeadState (EWC++ update, head types)

| Field | Value |
|---|---|
| **Phase** | PH45 — Mistake-Closure + Online Heads + Replay Buffer |
| **Stage** | S10 — Anneal + Intelligence Objective J |
| **Crate** | `calyx-anneal` |
| **Files** | `crates/calyx-anneal/src/learn/online_head.rs` (≤500) |
| **Depends on** | T02 (ReplayBuffer provides sampled batches for updates) |
| **Axioms** | A4, A14 |
| **PRD** | `dbprdplans/12 §3`, `dbprdplans/27 §5` |

## Goal

Implement `OnlineHeadState`: the set of small, trainable derived structures that
adapt online as new outcomes arrive — predictor head weights, calibration head
weights, and RRF/fusion weights. Each update uses an EWC++-style continual
update with a Fisher-diagonal regularizer to prevent catastrophic forgetting of
prior knowledge. Only these derived structures are updated; frozen lens weights
are never touched (enforced by `FrozenLensGuard`, T04). Every parameter update
is shadow-tested through the PH43 substrate before promotion.

## Build (checklist of concrete, code-level steps)

- [ ] `enum HeadKind { Predictor, Calibrator, FusionWeights }` — the three adaptable head types; all ≤1024 parameters each (enforced at construction).
- [ ] `struct OnlineHead { kind: HeadKind, params: Vec<f32>, fisher_diag: Vec<f32>, version: u64 }` — `fisher_diag` is the EWC++ importance estimate per parameter.
- [ ] `struct OnlineHeadState { heads: HashMap<HeadKind, OnlineHead>, substrate: Arc<AnnealSubstrate>, clock: Arc<dyn Clock> }`.
- [ ] `fn update(&mut self, batch: &[ReplayEntry], lr: f32, fisher_weight: f32) -> Result<HeadUpdateOutcome, CalyxError>` — computes gradient from batch losses; applies EWC++ update: `Δθ = −lr · (∇L + fisher_weight · F · (θ − θ_prior))`; wraps as an `AnnealAction`; passes through `substrate.propose_change`; on `Promote`: apply update, increment `version`; on `Revert`: discard.
- [ ] `fn predict(&self, cx: &Constellation) -> f64` — forward pass of the `Predictor` head; used by regression re-assert (T05).
- [ ] `fn calibrate(&self, raw_score: f64) -> f64` — Platt-scaling via the `Calibrator` head.
- [ ] `fn fusion_weights(&self) -> &[f32]` — returns current RRF/fusion weights for use by search fusion.
- [ ] Persist `OnlineHead` to `anneal_heads` CF after each promoted update.
- [ ] Parameter count enforced at construction: `params.len() > 1024` → `CALYX_ANNEAL_HEAD_TOO_LARGE`.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: initialize `Predictor` with all-zero params; apply one update with batch containing surprise `1.0`; params change from zero by a computable amount (verify exact first-update delta for lr=0.01, batch=1).
- [ ] unit: EWC++ regularizer prevents forgetting: apply update on task A, then update on task B with high fisher weight; task A loss does not increase by more than `fisher_weight × Σ F_i × Δθ_i²` (verify bound numerically).
- [ ] proptest: for any sequence of updates, `params.len()` remains constant (no growth/shrinkage).
- [ ] edge: empty batch → no parameter change; `params.len() = 1025` at construction → `CALYX_ANNEAL_HEAD_TOO_LARGE`; `lr=0.0` → params unchanged.
- [ ] fail-closed: substrate `propose_change` returns `Revert` → parameters NOT updated; `version` NOT incremented; `CALYX_ANNEAL_HEAD_UPDATE_REVERTED` returned to caller.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `anneal_heads` CF `version` field + `params` bytes.
- **Readback:** `calyx anneal head-status --kind Predictor` — prints `version`, `param_count`, `param_norm` (L2 of params).
- **Prove:** before update: `version=0`, `param_norm=0.0`; feed a high-surprise batch; after promoted update: `version=1`, `param_norm > 0`; `xxd anneal_heads` at the Predictor key shows updated bytes. No frozen lens hash changes (verified by `FrozenLensGuard` report in the same test run).

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH45 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
