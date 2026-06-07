# PH32 — Kernel-graph (~10%) + directed MFVS (~1%)

**Stage:** S6 — Lodestar Kernel  ·  **Crate:** `calyx-lodestar`  ·
**PRD roadmap:** P5  ·  **Axioms:** A10, A29

## Objective

Implement the staged approximate kernel-discovery pipeline inside `calyx-lodestar`:
condense the association graph (PH31 SCC), select the ~10% kernel-graph (high
in/out-degree + betweenness + low groundedness-distance + LP-relaxation rounding),
then run directed MFVS to find the ~1% grounding kernel. The MFVS uses the
LP-relaxation `O(log τ* log log τ*)`-approximation with local search, plus
tournament 2-approx and bounded-genus `O(g)`-approx specializations. The
approximation factor is reported (auditable, never asserted). An incremental
re-evaluation hook is exposed for Anneal (PH43+).

## Dependencies

- **Phases:** PH31 (SCC condensation, betweenness, LP scaffolding, `AssocGraph` —
  all required before kernel-graph selection can run)
- **Provides for:** PH33 (kernel index + kernel_answer uses the `~1%` member list),
  PH34 (multi-scope build_kernel calls this pipeline per scope),
  PH43 (Anneal's incremental re-eval hook)

## Current state (build off what exists)

`calyx-lodestar` is a 9-line stub (greenfield). PH31 delivers `AssocGraph`,
`tarjan_scc`, `betweenness`, and `LpProblem`. This phase seeds `dfvs.rs` and
`kernel_graph.rs` from the ContextGraph `context-graph-solver` source (copied into
`crates/calyx-lodestar/src/`, never linked).

## Deliverables (file plan, each ≤500 lines)

| File | Responsibility |
|---|---|
| `crates/calyx-lodestar/src/lib.rs` | crate root; re-exports `kernel_graph`, `dfvs`, `kernel`, `incremental` |
| `crates/calyx-lodestar/src/kernel_graph.rs` | `select_kernel_graph(graph, scc, betweenness, anchors, params) -> KernelGraph`; high in/out-degree + betweenness + low groundedness-distance + LP-relaxation rounding; target ~10% |
| `crates/calyx-lodestar/src/dfvs.rs` | `dfvs_approx(graph) -> DfvsResult`; LP-relaxation `O(log τ* log log τ*)`-approx + local search; `tournament_2approx(graph)`; `bounded_genus_approx(graph, genus)`; `approx_factor: f64` field |
| `crates/calyx-lodestar/src/kernel.rs` | `Kernel` struct (per PRD §6); `build_kernel_pipeline(graph, anchors, params) -> Kernel`; wires condense→kernel-graph→MFVS |
| `crates/calyx-lodestar/src/incremental.rs` | `IncrementalKernelEval`: delta-update hook for Anneal; accepts new/removed nodes/edges; re-runs only affected SCCs |

## Tasks (atomic — all must pass for the phase to be DONE)

| Card | Title | Depends |
|---|---|---|
| T01 | Kernel-graph selection: degree + betweenness + groundedness filter | — (needs PH31) |
| T02 | LP-relaxation rounding for kernel-graph (~10%) | T01 |
| T03 | MFVS LP-relaxation approx + local search (`dfvs_approx`) | T02 |
| T04 | Tournament 2-approx + bounded-genus O(g) specializations | T03 |
| T05 | `build_kernel_pipeline` wiring + `Kernel` struct + approx-factor reporting | T04 |
| T06 | Incremental re-eval hook for Anneal | T05 |

## FSV exit gate (the phase is DONE only when this is byte-proven on aiwonder)

On a **synthetic graph with a planted MFVS** (known set of feedback-vertex nodes):
1. `build_kernel_pipeline` on the synthetic graph finds the planted FVS members;
   read the computed `members: Vec<CxId>` from the `Kernel` struct (debug print or
   `calyx readback`).
2. Every planted FVS node appears in the computed members list; no planted non-FVS
   node appears (exact set recovery on small graphs, near-optimal on larger).
3. `approx_factor` is printed and ≤ the theoretical `O(log τ* log log τ*)` bound
   for the test graph size.
4. Evidence (stdout + planted vs computed table) attached to the PH32 GitHub issue.

## Risks / landmines

- **LP solver dependency:** the LP relaxation requires an external LP solver
  (e.g. `highs` via `good_lp`); pin the version and test on aiwonder where the
  library must be available. If the solver is unavailable, fall back to the
  greedy approximation with a `CALYX_KERNEL_LP_UNAVAILABLE` warning.
- **Approximation factor ≠ constant:** `O(log τ* log log τ*)` grows with the
  optimal FVS size τ*; report the actual factor for the corpus, never claim "2-approx"
  for the LP path.
- **kernel-graph size overshoot:** the ~10% target is a goal, not a hard cap;
  log the actual fraction and surface it in `kernel_health`.
- **Incremental correctness:** Anneal delta updates must not corrupt the SCC
  component assignments; restrict incremental to edge-weight changes first,
  then tackle topology changes.
