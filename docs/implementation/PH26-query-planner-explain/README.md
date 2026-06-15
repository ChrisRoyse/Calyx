# PH26 — Query planner + intent + explain

**Stage:** S4 — Sextant Search & Navigation  ·  **Crate:** `calyx-sextant`  ·
**PRD roadmap:** P3  ·  **Axioms:** A17, A16

## Objective

Auto-select fusion strategy by intent (overridable explicitly per A17) and
deliver full `explain` output. The planner classifies query intent into one of
the 14 ContextGraph weight profiles, maps it to a `FusionStrategy`, enforces
cost caps + timeouts (rejecting unbounded plans), and wires the reranker hook
(`:8089`, candidate text request-scoped, zeroizing-owned, and never persisted by
the product path). `explain=true`
returns the per-lens + provenance breakdown already built in PH24; the planner
adds intent label, strategy chosen, cost estimate, and timeout budget to the
`ExplainHit`. The FSV gate requires: intent auto-selects the right strategy
(verified per case on aiwonder), `explain=true` returns the full breakdown, and
an unbounded plan is rejected (`10 §7`, `17 §7.3`).

## Dependencies

- **Phases:** PH25 (Pipeline strategy + sparse lens — `FusionStrategy::Pipeline`
  exists), PH24 (all fusion strategies, `search()`, `ExplainHit`), PH21 (lens
  capability cards for cost estimation)
- **Provides for:** PH55 (universal query surface routes through the planner),
  PH62 (CLI `search` uses the planner by default), PH63 (MCP tool calls planner)

## Current state (build off what exists)

`calyx-sextant` has all four fusion strategies (SingleLens, RRF, WeightedRRF,
Pipeline), `search()`, planner intent classification, planner cost caps,
planner explain enrichment, reranker hooks, and `SlotIndexMap`. Post-sweep
hardening #282 fixed the remaining fail-closed planner blind spots: `k=0`,
no-lenses, ef/slot over-cap, and cost-cap overflow now return cataloged errors.
Post-sweep hardening #290 fixed the reranker wire contract: Calyx sends the
live TEI `texts` request field, parses `[{index, score}]` rank arrays back into
candidate order, and fails closed on non-2xx HTTP status instead of returning
mock scores. Post-sweep hardening #296 wires that client into
`SearchEngine::search_with_reranker` for Pipeline result ordering; the path
uses only request-scoped candidate text from the sparse stage-1 index and fails
closed on non-2xx, mismatched score vectors, missing candidate text, or
non-Pipeline reranker requests. Post-sweep hardening #325 wraps candidate text
as `Zeroizing<String>` when it leaves the sparse index, stores
`RerankRequest.candidates` as `Vec<Zeroizing<String>>`, and keeps the serialized
HTTP body in `Zeroizing<String>`. Scalar/anchor/metadata query filters from the
PRD are implemented by #297 as `QueryFilters`: scalar comparisons over
`Constellation.scalars`, anchor kind/value/source/confidence predicates, and
built-in metadata predicates over vault, modality, panel version, created time,
and input redaction/pointer. Arbitrary user metadata maps are not claimed here
because the core `Constellation` record does not yet expose a free-form metadata
map.
Post-sweep hardening #326 wires `PlannerExplain` into
`SearchEngine::planned_explain_search`, so planner intent/strategy/cost/timeout
and the executed provenanced hits are returned in one object.
Post-sweep hardening #327 makes planner index-size estimation ignore inactive
slots for default planned searches; parked/retired explicit slots still fail
closed in execution with `CALYX_SEXTANT_SLOT_INACTIVE`.

## Deliverables (file plan, each ≤500 lines)

| File | Responsibility |
|---|---|
| `crates/calyx-sextant/src/planner.rs` | intent classifier → strategy selection; cost model + caps; timeout enforcement |
| `crates/calyx-sextant/src/planner_explain.rs` | planner-enriched explain output: intent, strategy chosen, cost estimate, timeout, executed hits |
| `crates/calyx-sextant/src/reranker.rs` | reranker hook: HTTP call to :8089, request-scoped text, zeroizing candidate ownership, timeout |
| `crates/calyx-sextant/tests/query_filters_fsv.rs` | scalar/anchor/built-in metadata filter execution and readback |
| `crates/calyx-sextant/tests/reranker_search_fsv.rs` | SearchEngine Pipeline reranker ordering and request/response readback |
| `crates/calyx-sextant/tests/stage4_fsv.rs` | intent/strategy, Pipeline, reranker, explain, and unbounded-plan FSV |

## Tasks (atomic — all must pass for the phase to be DONE)

| Card | Title | Depends |
|---|---|---|
| T01 | Intent classifier (keyword rules → profile name) | — |
| T02 | Strategy selector + cost model | T01 |
| T03 | Cost caps + timeout enforcement (reject unbounded plans) | T02 |
| T04 | Reranker hook (`:8089`, Zeroizing, timeout) | T03 |
| T05 | Planner `explain` enrichment | T04 |
| T06 | Planner intent FSV: per-case + unbounded rejection | T05 |

## FSV exit gate (the phase is DONE only when this is byte-proven on aiwonder)

Run the Stage 4 FSV on aiwonder. The readback JSON must include:
- `planner_intent="Causal"` and `planner_strategy="weighted_rrf:causal"`.
- `unbounded="CALYX_SEXTANT_PLAN_UNBOUNDED"`.
- `rerank.scores` from the resident `:8089` TEI reranker in the Stage 4
  full-stack FSV readback.
- #296 `reranker-search-readback.json` is the controlled SearchEngine reranker
  FSV: it proves baseline order, reranked order, request text scope, and
  `pipeline+rerank` strategy through the real Calyx search path, but its
  captured request/response files are synthetic wire artifacts, not the
  resident `:8089` model readback.
- #325 `reranker-search-readback.json` showing
  `candidates_owned_by_zeroizing=true`, `serialized_body_zeroizing=true`,
  request text count/scope booleans, and `pipeline+rerank` strategy; the
  captured `reranker-http-request.txt` remains the separate synthetic wire SoT.
- #326 `planner-explain-readback.json` showing `intent`, `strategy`,
  `cost_estimate`, `timeout_ms`, `hit_explain_strategy`, and
  `hit_provenance_hex` from the same executed planned-search path.
- #297 `query-filter-readback.json` showing unfiltered ids, filtered ids,
  provenance hashes, and exclusion counts for scalar/anchor/metadata mismatches.
- `pipeline_subset_ok=true`.
- `pipeline_empty_stage1_hits=0`.

For #290 the readback root is
`/home/croyse/calyx/data/fsv-issue290-sextant-pipeline-reranker-20260608`.
For #296 the readback root is
`/home/croyse/calyx/data/fsv-issue296-reranker-search-20260608`.
For #325 the readback root is
`/home/croyse/calyx/data/fsv-issue325-reranker-candidate-privacy-20260608`.
For #326 the readback root is
`/home/croyse/calyx/data/fsv-issue326-planned-explain-path-20260608`.
For #297 the readback root is
`/home/croyse/calyx/data/fsv-issue297-query-filters-20260608`.

## Risks / landmines

- **Intent classifier accuracy**: the classifier uses keyword rules (not an ML
  model at this stage); it must be deterministic and tested per-case; false
  positives for "causal" queries are acceptable (conservative) — false negatives
  are not (a causal query that routes to general RRF loses the directional boost).
- **Cost cap calibration**: cost is estimated as `num_slots × index_size ×
  ef_factor`; the cap must be set conservatively (reject plans expected to exceed
  `p99 < 60 ms` for Pipeline per `10 §8`); recalibrate on aiwonder after first
  real workload (PH46 Anneal will automate this).
- **Reranker timeout**: the `:8089` GTE reranker on aiwonder may be unavailable
  during dev; the planner must fail-closed (`CALYX_SEXTANT_RERANKER_TIMEOUT`)
  and never silently skip reranking when the spec requests it.
- **A17 override**: user can always override the planner's choice via
  `Query.fusion = FusionStrategy::Explicit(...)` — the planner must check for
  an explicit override before auto-selecting and skip classification in that case.
