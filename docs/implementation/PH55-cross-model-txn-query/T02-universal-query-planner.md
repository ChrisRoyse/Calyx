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

- [x] Define `UniversalQuery` in `query/mod.rs`:
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
- [x] Define `PlanStep` enum:
  `RelationalScan { collection, filter, index: Option<IndexSpec> }` |
  `DocScan { collection, path_filter }` |
  `KvGet { ns, key }` |
  `TsRangeScan { series, start, end }` |
  `GraphHop { from_cx_ids, hop_kind }` |
  `VectorFusion { lens_ids, query_vec, limit }` |
  `Aggregate { spec }` |
  `Ask { question, context_cx_ids }`.
- [x] Define `CrossModelPlan { steps: Vec<PlanStep>, estimated_cost_ms: f32, explain: Option<ExplainOutput> }`.
- [x] Implement `plan(vault: &AsterVault, query: &UniversalQuery) -> Result<CrossModelPlan>`:
  - For each non-None query field, produce the appropriate `PlanStep`(s).
  - Estimate cost per step (conservative heuristics: relational full-scan = 50 ms
    per 100K rows; index scan = 5 ms; KV = 0.1 ms; TS range = 1 ms/1K points;
    graph hop = 10 ms/hop; vector ANN = 5 ms/lens; ASK = 200 ms).
  - Sum cost estimates; if `estimated_cost > cost_cap_ms` (or > vault threshold
    when no cap declared) → return `CALYX_PLANNER_COST_CAP` with the estimate.
  - If `explain=true`, populate `ExplainOutput` with per-step cost + chosen index.
  - Order steps: relational filter first (most selective); then graph hop; then
    vector/FTS; then aggregate; then ASK last (most expensive).
- [x] Vault threshold for "unbounded" rejection: `DEFAULT_COST_CAP_MS = 30_000`
  (30 s); configurable via `TxnPolicy`.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: `plan` for `relational + kv` query → `CrossModelPlan` with 2 steps in
  order `[RelationalScan, KvGet]`; `estimated_cost_ms > 0`.
- [x] unit: `plan` with `cost_cap_ms=Some(1)` and relational full-scan (est ~50ms)
  → `CALYX_PLANNER_COST_CAP`.
- [x] unit explain: `plan(explain=true)` → `ExplainOutput.steps` has one entry per
  `PlanStep` with non-zero cost; total = sum of parts.
- [x] unit unbounded rejection: `UniversalQuery { relational: Some(…), cost_cap_ms: None }`
  on a collection with 1M rows → estimated > `DEFAULT_COST_CAP_MS` →
  `CALYX_PLANNER_COST_CAP`.
- [x] proptest: for any query with `cost_cap_ms=Some(cap)`, if the planner accepts
  it, `estimated_cost_ms <= cap` (planner does not underestimate past the cap).
- [x] edge (≥3): (1) empty `UniversalQuery` (all None, no ask) → plan with 0 steps,
  `estimated_cost_ms=0`, accepted; (2) `ASK` only → plan has `Ask` step;
  (3) all modes set simultaneously → steps in correct dependency order.
- [x] fail-closed: `cost_cap_ms=Some(0)` → `CALYX_PLANNER_COST_CAP` immediately
  (any non-zero estimated cost exceeds cap).

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** durable Aster vault bytes on aiwonder plus planner readback JSON under
  `/home/croyse/calyx/data/fsv-issue464-query-planner-20260613T150006Z`.
- **Readback:**
  ```
  CALYX_FSV_ROOT=/home/croyse/calyx/data/fsv-issue464-query-planner-20260613T150006Z \
    cargo test -p calyx-sextant \
    query::planner::fsv_tests::issue464_query_planner_fsv_writes_readback_artifacts \
    -- --ignored --nocapture
  cat /home/croyse/calyx/data/fsv-issue464-query-planner-20260613T150006Z/issue464-query-planner-readback.json
  xxd -g 1 -l 160 /home/croyse/calyx/data/fsv-issue464-query-planner-20260613T150006Z/vault/cf/relational/*.sst
  ```
- **Prove:** readback JSON shows `before_relational_rows=0`,
  `after_relational_rows=2`, happy steps `[relational_scan, kv_get]`,
  explain costs `50.0 + 0.1 = 50.1`, empty query accepted at `0.0`, ASK-only
  planned as `ask`, cap-zero rejected with `CALYX_PLANNER_COST_CAP`, and
  1M-row unbounded estimate rejected with `CALYX_PLANNER_COST_CAP`. The
  relational SST hex contains keys
  `010970609d868f214400080000000000000001` and
  `010970609d868f214400080000000000000002`.

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] FSV evidence (readback output / screenshot) attached to the PH55 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
