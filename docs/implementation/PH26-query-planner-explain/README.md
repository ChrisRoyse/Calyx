# PH26 — Query planner + intent + explain

**Stage:** S4 — Sextant Search & Navigation  ·  **Crate:** `calyx-sextant`  ·
**PRD roadmap:** P3  ·  **Axioms:** A17, A16

## Objective

Auto-select fusion strategy by intent (overridable explicitly per A17) and
deliver full `explain` output. The planner classifies query intent into one of
the 14 ContextGraph weight profiles, maps it to a `FusionStrategy`, enforces
cost caps + timeouts (rejecting unbounded plans), and wires the reranker hook
(`:8089`, candidate text request-scoped and never persisted). `explain=true`
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

## Deliverables (file plan, each ≤500 lines)

| File | Responsibility |
|---|---|
| `crates/calyx-sextant/src/planner.rs` | intent classifier → strategy selection; cost model + caps; timeout enforcement |
| `crates/calyx-sextant/src/planner_explain.rs` | planner-enriched explain output: intent, strategy chosen, cost estimate |
| `crates/calyx-sextant/src/reranker.rs` | reranker hook: HTTP call to :8089, request-scoped text, Zeroizing, timeout |
| `crates/calyx-sextant/tests/planner_intent.rs` | intent→strategy correctness per case; unbounded plan rejection; explain output |

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

Run `cargo test -p calyx-sextant planner_intent -- --nocapture` on aiwonder.
Must print per-case lines:
```
intent=code strategy=single_lens:code_slot ok=true
intent=causal strategy=weighted_rrf:causal ok=true
intent=general strategy=rrf ok=true
unbounded_plan rejected=CALYX_SEXTANT_PLAN_UNBOUNDED ok=true
explain_breakdown non_empty=true intent_label_present=true
```
Screenshot or copy of all five lines attached to the PH26 GitHub issue.

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
