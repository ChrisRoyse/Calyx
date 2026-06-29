# PH32 T08 - LP/DFVS solver-contract honesty

| Field | Value |
|---|---|
| **Phase** | PH32 - Kernel-graph + directed MFVS |
| **Stage** | S6 - Lodestar Kernel |
| **Crate** | `calyx-lodestar`, `calyx-mincut` |
| **Files** | `crates/calyx-lodestar/src/kernel_graph.rs` (<=500), `crates/calyx-lodestar/src/dfvs.rs` (<=500), `crates/calyx-lodestar/tests/ph32_lodestar_tests.rs` (<=500) |
| **Depends on** | T02, T03 |
| **Axioms** | A10, A16 |
| **PRD** | `dbprdplans/08 section 3` |

## Goal

Make PH32's solver contract match the implementation. Calyx has LP model types,
a bounded exact MFVS solver for the explicit LP-round path, and an
injected-solution rounding seam. Fallback mode still fails loud; heuristic DFVS
is reported as heuristic, not as LP output.
The generic DFVS path must identify itself as exact/greedy local search rather
than LP local search.

## Status

Implemented in issue #329, then solver-wired in #1013. Historical aiwonder FSV readbacks live under
`/home/croyse/calyx/data/fsv-issue329-lp-dfvs-contract-20260608`.
Issue #645 extends this contract to tau/approximation readback honesty:
approximate paths use a cyclic-SCC lower-bound estimate, expose
`tau_star_exact`, and do not clamp observed bounds down to exact-looking `1.0`.
FSV root:
`/home/croyse/calyx/data/fsv-issue645-dfvs-honest-20260611T072428Z`.

## Build

- [x] Rename generic `DfvsMethod` from `LpLocalSearch` to
  `ExactOrGreedyLocalSearch`.
- [x] Wire `lp_round_kernel_graph` to the bounded exact `solve_mfvs_lp` path.
- [x] Reject the fallback flag with `CALYX_KERNEL_LP_UNAVAILABLE`; no heuristic graph is returned as LP output.
- [x] Expand PH32 readbacks with direct-solver output, fallback-flag error,
  malformed-solution errors, infeasible rounded-solution errors, and method provenance.
- [x] Update PH32 docs/task cards to avoid configured-solver and LP-bound claims.

## Tests

- [x] unit: strict LP path solves a supported cyclic graph.
- [x] unit: fallback flag fails closed with `CALYX_KERNEL_LP_UNAVAILABLE`.
- [x] unit: injected `LpSolution` rounding is verified against residual graph
  acyclicity before returning output.
- [x] unit: cyclic all-zero injected output returns `CALYX_KERNEL_LP_INFEASIBLE`.
- [x] unit: DFVS readback method/provenance no longer contains `LpLocalSearch`.
- [x] unit: exact and approximate DFVS paths produce distinguishable
  `approx_factor`, `tau_star_estimate`, and `tau_star_exact` readback.

## FSV

- **SoT:** `/home/croyse/calyx/data/fsv-issue329-lp-dfvs-contract-20260608`
  historically; current #1013 solver evidence is
  `issue1013-lp-mincut-solver-readback.json`.
- **Readbacks:** `ph32-lp-round-readback.json`, `ph32-dfvs-readback.json`, and
  `01-lp-dfvs-contract-test.out`.
- **Prove:** direct solver output exists for supported graphs, fallback-flag error
  exists, solver-limit failure does not return a heuristic selection, malformed
  injected values fail closed, and generic DFVS method/provenance is exact/greedy
  local search.

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) <= 500 lines
- [x] FSV evidence attached to #329 and #1013
- [x] docs and readbacks describe the bounded exact solver and fail-closed limits
