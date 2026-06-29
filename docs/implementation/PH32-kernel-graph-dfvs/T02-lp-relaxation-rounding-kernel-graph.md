# PH32 T02 - LP/MFVS rounder + injected-solution verification

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

Wire the explicit LP-rounding path to `calyx-mincut`'s bounded exact MFVS
solver, while keeping injected `LpSolution` handling fail-closed. Fallback mode
is disabled and still fails closed instead of returning the T01 heuristic
selection as LP output.

## Status

Implemented in #234-era PH32 work, contract-hardened in #329, and solver-wired
in #1013. Current FSV readbacks include `ph32-lp-round-readback.json` and
`issue1013-lp-mincut-solver-readback.json`.

## Build

- [x] `LpRoundParams { threshold, fallback_to_heuristic }`; fallback is disabled
  by default.
- [x] `lp_round_kernel_graph(...)` calls `calyx_mincut::solve_mfvs_lp` and rounds
  the returned solution.
- [x] `lp_round_kernel_graph_from_solution(...)` accepts only explicit
  `SolveStatus::Optimal`; `Infeasible` maps to `CALYX_KERNEL_LP_INFEASIBLE`.
  The injected objective must be finite and every value must be finite and in
  `[0,1]`, and `objective_value` must match `sum(values)`, otherwise
  `CALYX_KERNEL_INVALID_PARAMS`.
- [x] Injected solution rounding includes values `>= threshold`.
- [x] Rounded selections are verified as feedback vertex sets; cyclic all-zero
  output fails with `CALYX_KERNEL_LP_INFEASIBLE`.
- [x] DAGs may truthfully round to an empty FVS with `lp_fraction=0.0`.
- [x] `lp_round_kernel_graph(...)` with fallback enabled returns
  `CALYX_KERNEL_LP_UNAVAILABLE`; heuristic selections are not returned as LP
  output.

## Tests

- [x] unit: direct solver on a triangle returns the expected one-node minimum FVS.
- [x] unit: feasible injected values at threshold `0.5` round deterministically.
- [x] unit: fallback mode fails closed with `CALYX_KERNEL_LP_UNAVAILABLE`.
- [x] edge: all injected values below threshold on a cyclic graph fail closed as
  `CALYX_KERNEL_LP_INFEASIBLE`.
- [x] edge: NaN, infinite objective, and out-of-range injected values fail closed
  with `CALYX_KERNEL_INVALID_PARAMS`.
- [x] edge: mismatched objective value fails with `CALYX_KERNEL_INVALID_PARAMS`.
- [x] edge: cyclic graph over the exact solver bound fails with
  `CALYX_KERNEL_LP_UNAVAILABLE` carrying `CALYX_LP_SOLVER_LIMIT`.

## FSV

- **SoT:** `ph32-lp-round-readback.json` and
  `issue1013-lp-mincut-solver-readback.json`.
- **Readback:** read the JSON files after running the PH32 and issue1013 FSV
  tests with `CALYX_FSV_ROOT` set.
- **Prove:** JSON contains `contract=bounded_exact_mfvs_solver`, the triangle
  direct solver output, DAG empty-FVS output, cyclic all-zero infeasibility,
  fallback flag failure, malformed injected-solution failures, and solver-limit
  failure with no heuristic fallback.

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) <= 500 lines
- [x] FSV evidence attached to #329 and #1013
- [x] docs claim the bounded exact solver and its fail-closed limits
