# PH50 — Super-intelligence Predicate + `reverse_query`

**Stage:** S11 — Oracle & AGI Layer  ·  **Crate:** `calyx-oracle`  ·
**PRD roadmap:** `21 §3/§5`  ·  **Axioms:** A20, A23

## Objective

Implement two capabilities from the Royse corpus:
1. **`super_intelligence(domain)`** — the falsifiable 6-tier predicate that measures per-domain
   whether operational super-intelligence (in the paper's sense) has been reached, and if not,
   which tier fails and the cheapest fix. Each tier is measured against held-out oracle outcomes
   (Goodhart-defended). "Define super-intelligence as a benchmark and the system unlocks it for
   the domain when the benchmarks pass — as a query" (`21 §3`).
2. **`reverse_query(answer)`** — epistemic symmetry (A23, from *The Symmetry of Knowing*):
   given an answer/outcome, traverse the grounded association/causal graph *backwards* to
   recover the likely questions/causes. Grounded edges only; ungrounded traversal labeled
   `provisional`. Powers abductive reasoning, consequence-inversion, and grounding-gap discovery.

> Honesty is the feature: `super_intelligence` reports the failing tier + cheapest fix on a real
> domain; it never returns `true` when any tier fails. `reverse_query` on an ungrounded path
> returns `provisional`, never a fabricated confident cause.

**Current state:** `calyx-oracle` stub + PH49 modules exist; these modules are new additions.

## Dependencies

- **Phases:** PH49 (oracle_predict, OracleSelfConsistency, SufficiencyBound — tier 1 and 2 inputs),
  PH33 (kernel_answer + grounding_gaps — tier 3 kernel_exists check + recall measurement),
  PH30 (panel sufficiency — tier 2 panel_sufficient),
  PH38 (τ calibration — tier 4 calibrated),
  PH45 (mistake-closure — tier 6 mistake_closed),
  PH48 (J + Goodhart held-out — tier 5 goodhart_defended)
- **Provides for:** PH51 (`complete()` uses `reverse_query` back-edge traversal for abduction)

## Current state (build off what exists)

PH49 complete: `Prediction`, `OracleSelfConsistency`, `SufficiencyBound`, `check_sufficiency`,
`oracle_predict`, `expand`, `select` all exist. New files in this phase: `super_intel.rs`,
`reverse_query.rs`. No existing tier predicate logic.

## Deliverables (file plan, each ≤500 lines)

| File | Responsibility |
|---|---|
| `src/super_intel.rs` | `super_intelligence(domain)` — 6-tier predicate; `SuperIntelReport { tiers, failing_tier, cheapest_fix }` type; per-tier measurement against held-out oracle outcomes |
| `src/reverse_query.rs` | `reverse_query(vault, answer)` — asymmetric back-edge traversal; kernel-toward-antecedents; `provisional` tagging for ungrounded edges; returns `Vec<Cause>` |
| `src/types.rs` (extend) | Add `SuperIntelReport`, `Tier`, `TierResult`, `Cause` types |

## Tasks (atomic — all must pass for the phase to be DONE)

| Card | Title | Depends |
|---|---|---|
| T01 | `SuperIntelReport` types + 6-tier enum | — |
| T02 | `super_intelligence`: tiers 1–3 (oracle_clean, panel_sufficient, kernel_exists) | T01 |
| T03 | `super_intelligence`: tiers 4–6 (calibrated, goodhart_defended, mistake_closed) | T02 |
| T04 | `reverse_query`: back-edge traversal + provisional tagging | T01 |
| T05 | FSV: predicate reports failing tier; reverse recovers known cause | T03, T04 |

## FSV exit gate (the phase is DONE only when this is byte-proven on aiwonder)

1. On a real domain: `super_intelligence(domain)` reports `failing_tier` and `cheapest_fix`
   correctly; read the JSON output with `calyx readback super_intelligence <domain>`.
2. `reverse_query` on a known cause recovers it: seed a vault with a known `cause → effect`
   association; call `reverse_query(effect)`; the known cause appears in the returned list with
   `provisional: false` (grounded edge).
3. An ungrounded reverse query returns results with `provisional: true` — read the `provisional`
   field in the returned `Cause` structs.

## Risks / landmines

- **Tier 5 (Goodhart-defended):** requires held-out oracle outcomes from PH48 J machinery;
  do not measure Goodhart defense on training data — separate the held-out set rigorously.
- **Tier 3 (kernel_exists):** delegates to Lodestar PH33 kernel recall; the ≥0.95 threshold
  is from the stage file — do not soften it.
- **Back-edge traversal cycles:** same cycle-detection logic as butterfly tree (visited-set);
  `reverse_query` on a cyclic graph must terminate.
- **`provisional` discipline:** any back-edge without grounded recurrence backing must be
  `provisional`; mixing grounded and provisional in one result is fine but must be labeled per-edge.
