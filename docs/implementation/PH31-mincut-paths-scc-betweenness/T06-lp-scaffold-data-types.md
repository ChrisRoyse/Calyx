# PH31 · T06 — LP model + bounded MFVS solver

> **STATUS: ✅ DONE / FSV-signed-off.** Implemented in
> `crates/calyx-mincut/src/lp_scaffold.rs` with serializable LP variables,
> constraints, problems, solutions, validation, directed cycle-elimination
> constraints, bounded exact MFVS solving, and PH32-ready FVS verification.
> Current hardening issue: #1013. aiwonder/local FSV readback:
> `ph31-lp-readback.json`.

> Historical checklist note: the unchecked implementation prompts below were
> satisfied by the closed Stage 6 evidence; current state is the status/evidence
> block above.

| Field | Value |
|---|---|
| **Phase** | PH31 — mincut/paths: graph build + SCC + betweenness |
| **Stage** | S6 — Lodestar Kernel |
| **Crate** | `calyx-mincut` |
| **Files** | `crates/calyx-mincut/src/lp_scaffold.rs` (≤500) |
| **Depends on** | T03 (SCC types), T02 (`AssocGraph`) |
| **Axioms** | A10 |
| **PRD** | `dbprdplans/08 §3` (Stage 2: LP-relaxation rounding for kernel-graph; Stage 3: LP-relaxation MFVS) |

## Goal

Define the LP variable/constraint/solution data types that PH32's kernel-graph
selection and MFVS approximation will populate. The current implementation also
generates real directed cycle-elimination constraints and provides a bounded
exact MFVS solver used by Lodestar's explicit LP-round path. Solver limits fail
closed instead of falling back to heuristic output.

## Build (checklist of concrete, code-level steps)

- [ ] `pub struct LpVariable { id: usize, name: String, lb: f64, ub: f64 }` —
  lower/upper bounds; for MFVS each variable is in `[0.0, 1.0]`.
- [ ] `pub struct LpConstraint { coeffs: Vec<(usize, f64)>, sense: ConstraintSense, rhs: f64 }`
  where `ConstraintSense` is `Leq | Geq | Eq`.
- [ ] `pub struct LpProblem { vars: Vec<LpVariable>, constraints: Vec<LpConstraint>, objective: Vec<(usize, f64)>, sense: OptSense }`
  where `OptSense` is `Minimize | Maximize`.
- [ ] `pub struct LpSolution { values: Vec<f64>, objective_value: f64, status: SolveStatus }`
  where `SolveStatus` is `Optimal | Infeasible | Unbounded | NotSolved`.
- [ ] `pub fn mfvs_lp_problem(graph: &AssocGraph) -> LpProblem` — constructs the
  MFVS LP model: one variable `x_v ∈ [0,1]` per node, one constraint per directed
  cycle cover when bounded, and objective `minimize Σ x_v`.
- [ ] `pub fn solve_mfvs_lp(graph: &AssocGraph) -> LpSolution` — returns a binary
  optimal FVS for acyclic graphs and supported cyclic graphs; cyclic graphs over
  the bound fail with `CALYX_LP_SOLVER_LIMIT`.
- [ ] `pub fn verify_feedback_vertex_set(graph, members)` — verifies by rereading
  the residual graph state, not by trusting solver status alone.
- [ ] All types `#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]`.
- [ ] Validation: `LpProblem::validate()` → `CALYX_LP_INVALID` if any coefficient
  references an out-of-range variable index.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: construct `LpProblem` with 3 variables and 2 constraints; serialize to
  JSON and deserialize; round-trip byte-identical.
- [ ] unit: `mfvs_lp_problem` on the triangle graph `A→B→C→A` (3 nodes) →
  produces 3 variables, objective `[1.0, 1.0, 1.0]`, and one cycle constraint
  `x_A + x_B + x_C >= 1`.
- [ ] unit: `solve_mfvs_lp` on the triangle graph returns a one-node FVS and
  `verify_feedback_vertex_set` proves the residual graph is acyclic.
- [ ] unit: `solve_mfvs_lp` on a DAG returns an all-zero optimal solution.
- [ ] edge: `LpProblem::validate()` with constraint referencing variable index 5
  when only 3 variables exist → `CALYX_LP_INVALID`.
- [ ] edge: cyclic graph above the exact bound → `CALYX_LP_SOLVER_LIMIT`, no
  heuristic fallback.
- [ ] fail-closed: variable with `lb > ub` → `CALYX_LP_INVALID` on construction.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `cargo test -p calyx-mincut lp_scaffold -- --nocapture` stdout.
- **Readback:** `cargo test -p calyx-mincut lp_scaffold 2>&1 | tee /tmp/ph31_t06_fsv.txt && cat /tmp/ph31_t06_fsv.txt`.
- **Prove:** serde round-trip test passes (printed JSON matches re-parsed struct);
  triangle LP problem prints 3 variables with correct bounds and objective;
  all tests pass; output attached to PH31 GitHub issue.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH31 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
