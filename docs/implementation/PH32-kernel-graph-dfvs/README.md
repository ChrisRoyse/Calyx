# PH32 - Kernel-graph (~10% target) + directed MFVS (~1% target)

**Stage:** S6 - Lodestar Kernel  |  **Crate:** `calyx-lodestar`  |
**PRD roadmap:** P5  |  **Axioms:** A10, A29

## Objective

Implement the staged kernel-discovery pipeline inside `calyx-lodestar`: score
the association graph, select the ~10% kernel-graph, optionally round a supplied
LP solution, then run verified directed FVS selection to find the grounding
kernel. The current LP path is intentionally a scaffold: no external LP solver is
configured. Strict mode fails with `CALYX_KERNEL_LP_UNAVAILABLE`; fallback mode
returns the heuristic kernel-graph with that warning. Generic DFVS uses exact
search on small graphs or greedy local search on larger graphs, plus tournament
2-approx and bounded-genus specializations.

## Dependencies

- **Phases:** PH31 (SCC condensation, betweenness, LP scaffold types, `AssocGraph`)
- **Provides for:** PH33 (kernel index + `kernel_answer`), PH34 (multi-scope
  `build_kernel`), PH43 (Anneal incremental re-eval)

## Current state

DONE / FSV-signed-off on aiwonder. `calyx-lodestar` owns kernel-graph scoring,
LP-round scaffold handling with explicit fallback warnings, verified DFVS
members, specializations, the serializable `Kernel` pipeline, and the
incremental evaluation hook.

Base FSV root: `/home/croyse/calyx/data/fsv-ph32-20260608`.
Contract-hardening FSV root:
`/home/croyse/calyx/data/fsv-issue329-lp-dfvs-contract-20260608`.

The ContextGraph solver remains an allowed seed source per `19 section 6`, but
PH32 landed as Calyx-native Rust over PH31 graph/scaffold types. It does not
link or import the live ContextGraph project.

## Deliverables

| File | Responsibility |
|---|---|
| `crates/calyx-lodestar/src/lib.rs` | crate root; re-exports `kernel_graph`, `dfvs`, `kernel`, `incremental` |
| `crates/calyx-lodestar/src/kernel_graph.rs` | `select_kernel_graph(...) -> KernelGraph`; score-based top-fraction selection; LP-round scaffold/injected-solution adapter; strict unavailable or heuristic fallback when no solver is configured |
| `crates/calyx-lodestar/src/dfvs.rs` | `dfvs_approx(...) -> DfvsResult`; exact-or-greedy local search for the generic path; `tournament_2approx`; `bounded_genus_approx`; verified members and auditable method/approx fields |
| `crates/calyx-lodestar/src/kernel.rs` | `Kernel` struct; `build_kernel_pipeline(graph, anchors, params) -> Kernel`; wires selection -> LP scaffold/fallback -> DFVS |
| `crates/calyx-lodestar/src/incremental.rs` | `IncrementalKernelEval`; delta-update hook for Anneal |

## Tasks

| Card | Title | Depends | Status |
|---|---|---|---|
| T01 | Kernel-graph selection: degree + betweenness + groundedness filter | PH31 | FSV |
| T02 | LP-round scaffold + injected-solution rounding | T01 | FSV / #329 hardened |
| T03 | DFVS exact/greedy local search (`dfvs_approx`) | T02 | FSV / #329 hardened |
| T04 | Tournament 2-approx + bounded-genus O(g) specializations | T03 | FSV |
| T05 | `build_kernel_pipeline` wiring + `Kernel` struct + approx-factor reporting | T04 | FSV |
| T06 | Incremental re-eval hook for Anneal | T05 | FSV |
| T08 | LP/DFVS solver-contract honesty | T02, T03 | Done / #329 |

## FSV Exit Gate

On a synthetic graph with a planted FVS:
1. `build_kernel_pipeline` finds the known FVS members.
2. Removing computed members leaves an acyclic graph.
3. The emitted method and approximation fields describe the actual algorithm
   used; they do not claim a configured LP solver.
4. LP scaffold FSV proves both strict fail-loud
   `CALYX_KERNEL_LP_UNAVAILABLE` and heuristic fallback warning bytes.

Base readbacks live under `/home/croyse/calyx/data/fsv-ph32-20260608`.
Issue #329 readbacks live under
`/home/croyse/calyx/data/fsv-issue329-lp-dfvs-contract-20260608`.

## Risks / Landmines

- **LP solver dependency:** no external LP solver is configured today. Strict
  mode must fail loud; fallback must carry `CALYX_KERNEL_LP_UNAVAILABLE`.
- **Injected LP solutions are test adapters:** `lp_round_kernel_graph_from_solution`
  rounds a supplied `LpSolution`; it is not proof that Calyx solved LP
  constraints.
- **Approximation factor honesty:** generic exact/greedy local search reports an
  auditable method, factor, `tau_star_estimate`, and `tau_star_exact` certificate,
  not the PRD's future LP theoretical bound. Post-#645, heuristic paths use an
  independent cyclic-SCC lower bound and never self-certify by setting tau from
  `members.len()`. FSV root:
  `/home/croyse/calyx/data/fsv-issue645-dfvs-honest-20260611T072428Z`.
- **kernel-graph size overshoot:** the ~10% target is a goal; log the actual
  fraction and surface it in readbacks.
- **Incremental correctness:** Anneal deltas must not corrupt SCC/component
  assumptions; topology changes remain conservative.
