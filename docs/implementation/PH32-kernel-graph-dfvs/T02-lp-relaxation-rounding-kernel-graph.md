# PH32 T02 - LP-round scaffold + injected-solution rounding

| Field | Value |
|---|---|
| **Phase** | PH32 - Kernel-graph (~10% target) + directed MFVS (~1% target) |
| **Stage** | S6 - Lodestar Kernel |
| **Crate** | `calyx-lodestar` |
| **Files** | `crates/calyx-lodestar/src/kernel_graph.rs` (<=500) |
| **Depends on** | T01, PH31 LP scaffold types |
| **Axioms** | A10 |
| **PRD** | `dbprdplans/08 section 3` |

## Goal

Expose the LP-rounding seam without pretending a real solver is configured.
`lp_round_kernel_graph_from_solution` rounds an injected `LpSolution` for tests
and future solver adapters. `lp_round_kernel_graph` strict mode fails loud with
`CALYX_KERNEL_LP_UNAVAILABLE`; fallback mode is disabled and also fails closed
instead of returning the T01 heuristic selection as LP output.

## Status

Implemented in #234-era PH32 work and contract-hardened in #329. aiwonder FSV
readbacks:
`/home/croyse/calyx/data/fsv-issue329-lp-dfvs-contract-20260608/ph32-lp-round-readback.json`.

## Build

- [x] `LpRoundParams { threshold, fallback_to_heuristic }`; fallback is disabled
  by default.
- [x] `lp_round_kernel_graph_from_solution(...)` accepts only explicit
  `SolveStatus::Optimal`; `Infeasible` maps to `CALYX_KERNEL_LP_INFEASIBLE`.
- [x] Injected solution rounding includes values `>= threshold`.
- [x] Empty rounded selection fails closed with `CALYX_KERNEL_EMPTY_RESULT`.
- [x] `lp_round_kernel_graph(...)` with `fallback_to_heuristic=false` returns
  `CALYX_KERNEL_LP_UNAVAILABLE`.
- [x] `lp_round_kernel_graph(...)` with fallback enabled returns
  `CALYX_KERNEL_LP_UNAVAILABLE`; heuristic selections are not returned as LP
  output.

## Tests

- [x] unit: injected values `[0.9, 0.3, 0.7, 0.1]` at threshold `0.5` select
  nodes 0 and 2.
- [x] unit: strict mode without a configured solver returns
  `CALYX_KERNEL_LP_UNAVAILABLE`.
- [x] unit: fallback mode fails closed with `CALYX_KERNEL_LP_UNAVAILABLE`.
- [x] edge: all injected values below threshold fail closed as empty result.
- [x] fail-closed: infeasible injected solution returns
  `CALYX_KERNEL_LP_INFEASIBLE`.

## FSV

- **SoT:** `/home/croyse/calyx/data/fsv-issue329-lp-dfvs-contract-20260608/ph32-lp-round-readback.json`
- **Readback:** `cat` the JSON on aiwonder after running
  `cargo test -p calyx-lodestar lp_round_selects_solution_values_and_missing_solver_fails_closed -- --nocapture`.
- **Prove:** JSON contains `contract=lp_solver_unconfigured_scaffold`,
  `missing_solver_error=CALYX_KERNEL_LP_UNAVAILABLE`,
  `fallback_flag_error=CALYX_KERNEL_LP_UNAVAILABLE`, and
  `not_solved_error=CALYX_KERNEL_LP_UNAVAILABLE`.

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) <= 500 lines
- [x] FSV evidence attached to #329
- [x] docs do not claim a configured LP solver or real LP constraints
