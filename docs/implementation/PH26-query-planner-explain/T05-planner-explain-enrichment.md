# PH26 · T05 — Planner `explain` enrichment

| Field | Value |
|---|---|
| **Phase** | PH26 — Query planner + intent + explain |
| **Stage** | S4 — Sextant Search & Navigation |
| **Crate** | `calyx-sextant` |
| **Files** | `crates/calyx-sextant/src/planner_explain.rs` (≤500) |
| **Depends on** | T04 (this phase) · PH24 T06 (`ExplainHit`) |
| **Axioms** | A15, A17 |
| **PRD** | `dbprdplans/10 §1`, `dbprdplans/10 §5`, `dbprdplans/10 §7` |

## Goal

Extend `ExplainHit` with planner-level metadata: the detected intent label, the
strategy chosen (and whether it was auto-selected or overridden), the cost
estimate, and the timeout budget. This makes `explain=true` a full audit trail
from intent classification through fusion to each hit's provenance — the
complete picture an agent needs to trust a result.

## Current implementation note

Post-sweep #326 implements this as `SearchEngine::planned_explain_search(query,
planner) -> PlannerExplain`. It plans first, forces per-hit explain on the
planned query, executes that planned query through `SearchEngine::search`, and
returns one envelope containing intent, strategy, override flag, cost estimate,
timeout, and the executed provenanced hits. `SearchEngine::planned_search`
provides the planned non-envelope path for callers that only need `Vec<Hit>`.

## Build (checklist of concrete, code-level steps)

- [x] `PlannerExplainHit` struct (wraps `ExplainHit`):
  ```rust
  pub struct PlannerExplainHit {
      pub inner: ExplainHit,
      pub intent: IntentLabel,
      pub strategy_chosen: String,        // human-readable, e.g. "weighted_rrf:causal"
      pub override_used: bool,
      pub cost_estimate: CostEstimate,
      pub timeout_budget_ms: u64,
  }
  ```
- [x] `fn planned_explain_search(query: &Query, map: &SlotIndexMap, embedder: &dyn EmbedQuery, ledger: &dyn LedgerProvider, clock: &dyn Clock, planner_config: &PlannerConfig) -> Result<Vec<PlannerExplainHit>, CalyxError>`:
      1. `plan(query, map)` → `PlannerOutput` (includes cost, timeout, strategy)
      2. Override `query.fusion` with `planner_output.strategy` if not overridden
      3. Call `explain_search()` from PH24 T06 → `Vec<ExplainHit>`
      4. For each `ExplainHit`, wrap into `PlannerExplainHit` with planner metadata
- [x] `PlannerExplainHit` derives `serde::Serialize` for JSON output
- [x] `strategy_chosen` string format: `"<strategy_name>[:<profile_name>]"` e.g.
      `"weighted_rrf:causal"`, `"single_lens:slot_0"`, `"rrf"`, `"pipeline"`
- [x] `explain=false` path still goes through the planner (for cost-cap checking)
      but returns plain `Vec<Hit>` from the fast `search()` path, not
      `Vec<PlannerExplainHit>` — the explain enrichment is only built on demand

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: `planned_explain_search` with `explain=true` → all hits have
      `intent` non-default, `strategy_chosen` non-empty, `cost_estimate.num_slots ≥ 1`
- [x] unit: code-intent query → `intent=Code strategy_chosen.starts_with("single_lens")` or `"rrf"` (fallback)
- [x] unit: explicit override → `override_used=true`
- [x] unit: `PlannerExplainHit` serializes to valid JSON (serde round-trip)
- [x] unit: planned causal query returns `PlannerExplain` with
      `intent=Causal`, `strategy=weighted_rrf:causal`, nonzero cost, timeout,
      hit explain strategy, and nonzero provenance
- [x] unit: `explain=false` path → returns `Vec<Hit>` (not `Vec<PlannerExplainHit>`);
      confirm by checking return type at compile time
- [x] edge: empty result set → `Ok(vec![])` with no panic
- [x] fail-closed: plan rejection (from T03) propagates through
      `planned_explain_search` → the error is returned, no partial hits

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** test output of `cargo test -p calyx-sextant planner_explain -- --nocapture`
- **Readback:** `cargo test -p calyx-sextant planner_explain -- --nocapture 2>&1`
- **Prove:** test prints for each case:
  `intent=Causal strategy=weighted_rrf:causal override=false cost_slots=N timeout=5000 explain_len=K`
  where K matches the number of hits returned; one such line is captured as
  FSV evidence
- **Post-sweep #326 SoT:**
  `/home/croyse/calyx/data/fsv-issue326-planned-explain-path-20260608/planner-explain-readback.json`
  proves `intent="Causal"`, `strategy="weighted_rrf:causal"`,
  `timeout_ms=5000`, `hit_provenance_nonzero=true`, and
  `planned_strategy_matches_hit_explain=true`.

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] FSV evidence (readback output / screenshot) attached to the PH26 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
