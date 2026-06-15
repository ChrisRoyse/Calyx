# PH46 · T04 — Loom materialization scope tuner

| Field | Value |
|---|---|
| **Phase** | PH46 — Autotune Loops |
| **Stage** | S10 — Anneal + Intelligence Objective J |
| **Crate** | `calyx-anneal` |
| **Files** | `crates/calyx-anneal/src/tune/scope_loom.rs` (≤500) |
| **Depends on** | T01 (ConfigBandit), PH27 (agreement cross-terms — materialization plan re-fit here), PH30 (panel sufficiency — `bits_per_anchor` drives materialization priority) |
| **Axioms** | A14 |
| **PRD** | `dbprdplans/12 §4` |

## Goal

Implement `LoomScopeTuner`: re-fits the Loom materialization plan (which cross-
terms are eagerly materialized vs computed lazily on demand, and which Concat
keys get an index) as query patterns and per-lens `bits_per_anchor` shift. A
cross-term pair `(L_i, L_j)` should be materialized eagerly iff it is queried
frequently AND carries grounded bits beyond either lens alone. The bandit drives
the plan change; win = lower average cross-term query latency with no reduction
in `bits_per_anchor` for the pair.

## Build (checklist of concrete, code-level steps)

- [ ] `struct MatPlanConfig { eager_pairs: Vec<(LensId, LensId)>, indexed_concat_keys: Vec<ConcatKey> }` — serializable as CBOR for `ConfigVariant`.
- [ ] `struct LoomScopeTuner { bandit: ConfigBandit, current_plan: MatPlanConfig, assay: Arc<dyn AssayMetrics>, loom: Arc<dyn LoomMaterializer>, substrate: Arc<AnnealSubstrate> }` — single bandit (plan-level, not pair-level, to keep arm count bounded).
- [ ] `fn evaluate_plan(plan: &MatPlanConfig, query_log: &QueryLog, assay: &dyn AssayMetrics) -> PlanScore { avg_latency_ns, bits_sum }` — runs the plan against a sample of recent queries; computes average cross-term latency and sum of `bits_per_anchor` for eager pairs.
- [ ] `fn generate_candidate_plan(current: &MatPlanConfig, assay: &dyn AssayMetrics, query_log: &QueryLog) -> MatPlanConfig` — adds the highest-bits pair not yet eager; drops the lowest-bits eager pair if over budget; re-sorts `indexed_concat_keys` by query frequency.
- [ ] `fn on_query_tick(&mut self, query_log: &QueryLog)` — on each N-query tick, evaluate the bandit; if exploring, generate a candidate plan and schedule a shadow evaluation via `substrate.propose_change`; `win = candidate_score.avg_latency_ns < incumbent_score.avg_latency_ns AND candidate_score.bits_sum >= incumbent_score.bits_sum`.
- [ ] Materialization changes are non-destructive: lazy→eager adds a cache entry; eager→lazy removes the cache but never deletes base data (A15).

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: `generate_candidate_plan` with two pairs `(L1,L2)` at `0.3 bits` and `(L2,L3)` at `0.1 bits`; candidate adds `(L1,L2)` first (higher bits).
- [ ] unit: candidate plan has lower latency AND equal bits → win; `incumbent_plan` updated; Ledger has `AutotunePromote`.
- [ ] unit: candidate plan has lower latency BUT lower bits (bits dropped by `0.02`) → NOT promoted; plan unchanged.
- [ ] proptest: for any plan evaluation, `bits_sum` for the incumbent plan is non-decreasing over promotions (materialization only adds grounded bits).
- [ ] edge: zero cross-term pairs → `generate_candidate_plan` returns current plan unchanged; single pair → bandit has one arm; plan with all pairs eager and no budget → candidate drops lowest-bits pair first.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** Loom's live materialization plan + Ledger `AutotunePromote` entries.
- **Readback:** `calyx anneal autotune-report --scope loom` — prints `eager_pairs` count, `bits_sum`, `avg_latency_ns`, recent plan promotions.
- **Prove:** run 20 `on_query_tick` calls with a synthetic query log showing `(L1,L2)` queried 90% of the time and carrying `0.4 bits`; confirm the materialization plan includes `(L1,L2)` as eager; `autotune-report` shows the promotion; `bits_sum` does not decrease across the run.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH46 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
