# PH48 — J Objective + Growth Curve + Intelligence Report

**Stage:** S10 — Anneal + Intelligence Objective J  ·  **Crate:** `calyx-anneal`  ·
**PRD roadmap:** `27` (all)  ·  **Axioms:** A32, A2, A8

## Objective

Implement the measurable composite `J` that Calyx maximizes, the intelligence-
gradient priority queue (`gradient.rs`) that picks the next highest-`ΔJ/cost`
action, the `intelligence_report` and `growth_curve` that audit and visualize
progress, and the full Goodhart defense (held-out validation + `Gτ` +
cross-lens anomaly checks) that ensures `J` is a real grounded measure and not
a gamed proxy. This is the cap of Stage 10 and the system's measurable,
DPI-capped, penalty-guarded drive toward maximum grounded intelligence.

## Dependencies

- **Phases:** PH47 (lens proposal — the `propose_lens` action is one entry in
  the gradient priority queue; sufficiency terms depend on it)
- **Provides for:** PH49 (Oracle consequence prediction gates on `J`-driven
  sufficiency state), PH70 (intelligence validation on real corpora uses the
  `growth_curve` and `intelligence_report` from this phase)

## Current state (build off what exists)

`calyx-anneal` crate: PH43–PH47 complete. No `J` composite, no gradient queue,
no `intelligence_report`, no `growth_curve` implementation. Greenfield. Source
in `dbprdplans/27` defines the composite formula and all terms.

**J composite formula (from `27 §2`, verbatim — no-compress):**
```
J(vault) =
    w1 · Σ_anchor  I(panel ; anchor)
  + w2 · n_eff
  + w3 · Σ_domain panel_sufficiency(domain)
  + w4 · kernel_recall (kernel-only / full)
  + w5 · oracle_accuracy − w6 · mistake_rate
  + w7 · meaning_compression_yield
  + w8 · coverage(domains, modalities)
  − P_redundant − P_ungrounded − P_goodhart
```
**Anneal invariants (binding):**
- Every `+` term is a real, grounded measurement; DPI ceiling (A8) caps info terms.
- `P_ungrounded`: bits about auto-labeled / ungrounded targets are tagged
  `provisional` and excluded from `J` entirely.
- `P_goodhart`: improvement that fails held-out validation or `Gτ` /
  cross-lens anomaly checks is penalized and the promotion reverted.
- No data deleted to optimize `J` (A15).
- Growth loop runs in bounded background budget (A26).

## Deliverables (file plan, each ≤500 lines)

| File | Responsibility |
|---|---|
| `src/j/j_composite.rs` | `J.rs` — compute all 8 `+` terms + 3 penalties; DPI ceiling; provisional exclusion |
| `src/j/gradient.rs` | `gradient.rs` — `ΔJ/cost` priority queue; `next_best_action`; `set_objective_weights` |
| `src/j/goodhart.rs` | Goodhart defense: held-out validation + `Gτ` check + cross-lens anomaly |
| `src/j/intelligence_report.rs` | `intelligence_report`: per-term breakdown, DPI headroom, provisional excluded count, gradient top-5 |
| `src/j/growth_curve.rs` | `growth_curve`: J-over-time series, persistence, monotone-rise check |

## Tasks (atomic — all must pass for the phase to be DONE)

| Card | Title | Depends |
|---|---|---|
| T01 | J composite computation (all terms + penalties + DPI cap) | — |
| T02 | Goodhart defense (held-out + Gτ + cross-lens anomaly) | T01 |
| T03 | Gradient priority queue (`ΔJ/cost`, `next_best_action`) | T01 |
| T04 | Intelligence report (per-term breakdown, DPI headroom) | T01, T02, T03 |
| T05 | Growth curve (J-over-time, rise check, persistence) | T01, T04 |
| T06 | Integration FSV: growth rises on real corpus; gamed change fails held-out | T01–T05 |

## FSV exit gate (the phase is DONE only when this is byte-proven on aiwonder)

Three proofs on aiwonder:
1. `J` is measured: `calyx anneal intelligence-report` prints a valid `J` value
   with per-term breakdown; DPI headroom shown; provisional terms excluded.
2. Growth curve rises: ingest 10k real-corpus documents under the autotune +
   mistake-closure loop; `calyx anneal growth-curve` shows a monotone-rising
   `J` series over time.
3. Gamed change rejected: inject a change that artificially inflates `w1·I` by
   adding a correlated lens (bypassing PH47 gate); Goodhart check detects the
   held-out regression; promotion is reverted; Ledger has `GoodharFailed` entry.

## Risks / landmines

- **DPI ceiling is the hardest constraint**: every info term must be passed
  through `min(term, DPI_ceiling)` where `DPI_ceiling = I(panel; reality)`
  measured on a grounded anchor set. Over-ceiling values silently inflate `J`
  without raising real intelligence.
- **`P_goodhart` computation** requires a held-out grounded set that is NOT
  used for training or autotuning; this set must be reserved at vault creation
  and never used for parameter estimation.
- **Weight tuning** (`set_objective_weights`) allows per-project calibration;
  but weights must not be set to zero for any term that is non-trivially
  measured (doing so would hide intelligence information from the report).
- **Growth curve monotone rise** is a soft expectation, not a hard invariant
  (J can dip on a bad heal or a data quality issue); the check flags non-
  monotone regions for investigation, not auto-revert.
- **No data deleted**: `J` penalties cannot be reduced by deleting data;
  `P_redundant` is reduced by parking/pruning lenses, not deleting constellations.
