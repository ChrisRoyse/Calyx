# PH32 T03 - DFVS exact/greedy local search (`dfvs_approx`)

| Field | Value |
|---|---|
| **Phase** | PH32 - Kernel-graph (~10% target) + directed MFVS (~1% target) |
| **Stage** | S6 - Lodestar Kernel |
| **Crate** | `calyx-lodestar` |
| **Files** | `crates/calyx-lodestar/src/dfvs.rs` (<=500) |
| **Depends on** | T02 kernel-graph input |
| **Axioms** | A10 |
| **PRD** | `dbprdplans/08 section 3` |

## Goal

Implement the verified generic directed-FVS path honestly: exact minimum FVS
search for small graphs, greedy removal for larger graphs, and local-search
shrink in both cases. The method is `ExactOrGreedyLocalSearch`; it is not an LP
relaxation solver and does not claim the future LP theoretical bound.

## Status

Implemented in PH32 and contract-hardened in #329. aiwonder FSV readbacks:
`/home/croyse/calyx/data/fsv-issue329-lp-dfvs-contract-20260608/ph32-dfvs-readback.json`.
Issue #645 hardened the approximation evidence contract: exact search and tight
lower-bound certificates report `tau_star_exact=true` and `approx_factor=1.0`;
heuristic paths report `tau_star_estimate` from an independent cyclic-SCC lower
bound, set `tau_star_exact=false` when the lower bound is not tight, and never
clamp the observed bound down to a nicer theoretical label.
aiwonder FSV root:
`/home/croyse/calyx/data/fsv-issue645-dfvs-honest-20260611T072428Z`;
primary readback `ph32-dfvs-honest-bounds-readback.json` SHA-256
`82617d924c8e8c47355cbc3dda83b75f27a47fb4a15f690bc983f8e4760322f7`.

## Build

- [x] `DfvsMethod::ExactOrGreedyLocalSearch`.
- [x] `dfvs_approx(kernel_graph)` dispatches tournament and bounded-genus
  specializations first, otherwise uses exact/greedy local search.
- [x] Exact search is bounded to small graphs; larger graphs use deterministic
  greedy FVS followed by local-search shrink.
- [x] Verify every result by removing members and confirming the graph is acyclic.
- [x] Fail closed with `CALYX_DFVS_VERIFICATION_FAILED` if verification fails.
- [x] Empty graph returns an empty result with method
  `ExactOrGreedyLocalSearch`.
- [x] Approximation readback distinguishes exact tau certificates from
  lower-bound estimates with `tau_star_exact`.

## Tests

- [x] unit: triangle returns exactly one member.
- [x] unit: planted synthetic graph includes both planted FVS nodes.
- [x] unit: DAG returns no members.
- [x] unit: readback records estimator provenance/method names for triangle,
  planted, and DAG cases.
- [x] proptest: tournament dispatch remains `Tournament2Approx`.

## FSV

- **SoT:** `/home/croyse/calyx/data/fsv-issue329-lp-dfvs-contract-20260608/ph32-dfvs-readback.json`
  and `/home/croyse/calyx/data/fsv-issue645-dfvs-honest-20260611T072428Z/ph32-dfvs-honest-bounds-readback.json`.
- **Readback:** `cat` the JSON on aiwonder after running
  `cargo test -p calyx-lodestar dfvs_triangle_planted_and_dag_cases_are_verified -- --nocapture`.
- **Prove:** computed members match the planted cases, method/provenance strings
  no longer contain `LpLocalSearch`, and exact vs approximate paths carry
  distinct `tau_star_estimate` / `tau_star_exact` evidence.

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) <= 500 lines
- [x] FSV evidence attached to #329
- [x] docs do not claim the generic DFVS path is LP-based
