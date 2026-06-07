# PH31 · T06 — LP scaffolding data types

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
selection and MFVS approximation will populate. This card is data-types-only
(no solver); the structures must be correct, serializable, and ready for PH32
to wire to an actual LP solver. Having them in PH31 lets PH32 focus on the
algorithm rather than type plumbing.

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
  LP relaxation for MFVS: one binary variable `x_v ∈ [0,1]` per node, one
  constraint per directed cycle cover; minimize `Σ x_v`. For PH31 this can
  use a cycle-enumeration stub that returns an empty constraint set (solver is PH32).
- [ ] All types `#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]`.
- [ ] Validation: `LpProblem::validate()` → `CALYX_LP_INVALID` if any coefficient
  references an out-of-range variable index.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: construct `LpProblem` with 3 variables and 2 constraints; serialize to
  JSON and deserialize; round-trip byte-identical.
- [ ] unit: `mfvs_lp_problem` on the triangle graph `A→B→C→A` (3 nodes) →
  produces an `LpProblem` with 3 variables (`x_A, x_B, x_C`), each in `[0,1]`;
  objective = `[1.0, 1.0, 1.0]` (minimize sum); status = `NotSolved`.
- [ ] unit: `LpSolution { values: [0.0, 1.0, 0.5], ... }` round-trips via serde.
- [ ] edge: `LpProblem::validate()` with constraint referencing variable index 5
  when only 3 variables exist → `CALYX_LP_INVALID`.
- [ ] edge: empty `LpProblem` (0 vars, 0 constraints) → `validate()` passes (trivially
  feasible); `status = NotSolved`.
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
