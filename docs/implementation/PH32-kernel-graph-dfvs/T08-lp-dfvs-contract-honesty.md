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

Make PH32's solver contract match the implementation. Calyx has LP scaffold
types and an injected-solution rounding seam, but no configured external LP
solver. Strict mode must fail loud; fallback mode must be visibly heuristic.
The generic DFVS path must identify itself as exact/greedy local search rather
than LP local search.

## Status

Implemented in issue #329. aiwonder FSV readbacks live under
`/home/croyse/calyx/data/fsv-issue329-lp-dfvs-contract-20260608`.
Issue #645 extends this contract to tau/approximation readback honesty:
approximate paths use a cyclic-SCC lower-bound estimate, expose
`tau_star_exact`, and do not clamp observed bounds down to exact-looking `1.0`.
FSV root:
`/home/croyse/calyx/data/fsv-issue645-dfvs-honest-20260611T072428Z`.

## Build

- [x] Rename generic `DfvsMethod` from `LpLocalSearch` to
  `ExactOrGreedyLocalSearch`.
- [x] Keep `lp_round_kernel_graph` strict fail-loud when no solver is configured.
- [x] Keep heuristic fallback explicit with `CALYX_KERNEL_LP_UNAVAILABLE`.
- [x] Expand PH32 readbacks with strict error, fallback-is-heuristic, and method
  provenance.
- [x] Update PH32 docs/task cards to avoid configured-solver and LP-bound claims.

## Tests

- [x] unit: strict LP path returns `CALYX_KERNEL_LP_UNAVAILABLE`.
- [x] unit: fallback path returns the heuristic selected set and warning bytes.
- [x] unit: injected `LpSolution` rounding is labeled as test-provided input, not
  solver output.
- [x] unit: DFVS readback method/provenance no longer contains `LpLocalSearch`.
- [x] unit: exact and approximate DFVS paths produce distinguishable
  `approx_factor`, `tau_star_estimate`, and `tau_star_exact` readback.

## FSV

- **SoT:** `/home/croyse/calyx/data/fsv-issue329-lp-dfvs-contract-20260608`
- **Readbacks:** `ph32-lp-round-readback.json`, `ph32-dfvs-readback.json`, and
  `01-lp-dfvs-contract-test.out`.
- **Prove:** strict unavailable error exists, fallback warning exists, fallback
  selection matches the heuristic selection, and generic DFVS method/provenance
  is exact/greedy local search.

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) <= 500 lines
- [x] FSV evidence attached to #329
- [x] docs and readbacks do not overclaim a real LP solver
