# PH55 · T02 — Universal query struct + planner: cross-model plan + reject unbounded

| Field | Value |
|---|---|
| **Phase** | PH55 — Cross-model transactions + universal query surface |
| **Stage** | S12 — Universal data layer |
| **Crate** | `calyx-sextant` |
| **Files** | `crates/calyx-sextant/src/query/mod.rs` (≤500), `crates/calyx-sextant/src/query/planner.rs` (≤500) |
| **Depends on** | PH26 T04 (query planner + intent), PH53 T01 (Collection, CollectionMode), PH54 T01 (IndexSpec) |
| **Axioms** | A16, A17, A19 |
| **PRD** | `dbprdplans/20 §4`, `dbprdplans/10 §0` |

## Goal

Define the `UniversalQuery` struct that expresses any combination of query
modes in one statement, and extend the PH26 planner to produce a
`CrossModelPlan` — an ordered sequence of `PlanStep`s (one per mode segment)
that the executor (T03) will run. An unbounded plan (no `cost_cap_ms` and
estimated cost > a vault-level threshold) is **rejected** before execution with
`CALYX_PLANNER_COST_CAP`. The planner returns an `Explain` breakdown when
`explain=true`.

## Build (checklist of concrete, code-level steps)

- [ ] Define `UniversalQuery` in `query/mod.rs`:
  ```rust
  pub struct UniversalQuery {
      pub relational: Option<RelationalFilter>,   // typed predicates on a Collection
      pub document:   Option<DocFilter>,          // path + value predicates
      pub kv:         Option<KvLookup>,           // point lookup by ns+key
      pub timeseries: Option<TsRange>,            // series + time range
      pub graph_hop:  Option<GraphHop>,           // association graph traversal
      pub vector:     Option<VectorQuery>,        // multi-lens ANN / FTS fusion
      pub aggregate:  Option<AggSpec>,            // count/sum/min/max/avg over results
      pub ask:        Option<AskSpec>,            // natural-language ASK over all above
      pub cost_cap_ms: Option<u32>,               // planner rejects if estimated > cap
      pub explain:     bool,
      pub isolation:   IsolationLevel,
  }
  ```
- [ ] Define `PlanStep` enum:
  `RelationalScan { collection, filter, index: Option<IndexSpec> }` |
  `DocScan { collection, path_filter }` |
  `KvGet { ns, key }` |
  `TsRangeScan { series, start, end }` |
  `GraphHop { from_cx_ids, hop_kind }` |
  `VectorFusion { lens_ids, query_vec, limit }` |
  `Aggregate { spec }` |
  `Ask { question, context_cx_ids }`.
- [ ] Define `CrossModelPlan { steps: Vec<PlanStep>, estimated_cost_ms: f32, explain: Option<ExplainOutput> }`.
- [ ] Implement `plan(vault: &AsterVault, query: &UniversalQuery) -> Result<CrossModelPlan>`:
  - For each non-None query field, produce the appropriate `PlanStep`(s).
  - Estimate cost per step (conservative heuristics: relational full-scan = 50 ms
    per 100K rows; index scan = 5 ms; KV = 0.1 ms; TS range = 1 ms/1K points;
    graph hop = 10 ms/hop; vector ANN = 5 ms/lens; ASK = 200 ms).
  - Sum cost estimates; if `estimated_cost > cost_cap_ms` (or > vault threshold
    when no cap declared) → return `CALYX_PLANNER_COST_CAP` with the estimate.
  - If `explain=true`, populate `ExplainOutput` with per-step cost + chosen index.
  - Order steps: relational filter first (most selective); then graph hop; then
    vector/FTS; then aggregate; then ASK last (most expensive).
- [ ] Vault threshold for "unbounded" rejection: `DEFAULT_COST_CAP_MS = 30_000`
  (30 s); configurable via `TxnPolicy`.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: `plan` for `relational + kv` query → `CrossModelPlan` with 2 steps in
  order `[RelationalScan, KvGet]`; `estimated_cost_ms > 0`.
- [ ] unit: `plan` with `cost_cap_ms=Some(1)` and relational full-scan (est ~50ms)
  → `CALYX_PLANNER_COST_CAP`.
- [ ] unit explain: `plan(explain=true)` → `ExplainOutput.steps` has one entry per
  `PlanStep` with non-zero cost; total = sum of parts.
- [ ] unit unbounded rejection: `UniversalQuery { relational: Some(…), cost_cap_ms: None }`
  on a collection with 1M rows → estimated > `DEFAULT_COST_CAP_MS` →
  `CALYX_PLANNER_COST_CAP`.
- [ ] proptest: for any query with `cost_cap_ms=Some(cap)`, if the planner accepts
  it, `estimated_cost_ms <= cap` (planner does not underestimate past the cap).
- [ ] edge (≥3): (1) empty `UniversalQuery` (all None, no ask) → plan with 0 steps,
  `estimated_cost_ms=0`, accepted; (2) `ASK` only → plan has `Ask` step;
  (3) all modes set simultaneously → steps in correct dependency order.
- [ ] fail-closed: `cost_cap_ms=Some(0)` → `CALYX_PLANNER_COST_CAP` immediately
  (any non-zero estimated cost exceeds cap).

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `cargo test -p calyx-sextant query::planner` output on aiwonder.
- **Readback:**
  ```
  cargo test -p calyx-sextant query -- --nocapture 2>&1 | tail -30
  calyx query --vault /home/croyse/calyx/test-vault \
    --filter 'orders.qty >= 1' --kv 'ns=1,key=sess' --explain
  ```
- **Prove:** `--explain` output lists 2 steps with non-zero cost estimates;
  rejection test prints `CALYX_PLANNER_COST_CAP` with the estimate.
  Evidence posted to PH55 issue.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH55 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
