# PH32 · T02 — LP-relaxation rounding for kernel-graph (~10%)

> **STATUS: ✅ DONE / FSV-signed-off.** Implemented in
> `crates/calyx-lodestar/src/kernel_graph.rs` with inclusive threshold rounding
> from `LpSolution`, empty-result fail-closed behavior, and explicit
> `CALYX_KERNEL_LP_UNAVAILABLE` fallback warnings when no external solver is
> configured. aiwonder FSV readback: `ph32-lp-round-readback.json`.

| Field | Value |
|---|---|
| **Phase** | PH32 — Kernel-graph (~10%) + directed MFVS (~1%) |
| **Stage** | S6 — Lodestar Kernel |
| **Crate** | `calyx-lodestar` |
| **Files** | `crates/calyx-lodestar/src/kernel_graph.rs` (≤500) |
| **Depends on** | T01 (score-based kernel-graph selection), PH31-T06 (`LpProblem`, `LpSolution`) |
| **Axioms** | A10 |
| **PRD** | `dbprdplans/08 §3` (Stage 2: LP-relaxation rounding for kernel-graph) |

## Goal

Augment kernel-graph selection with LP-relaxation rounding: solve a fractional LP
that assigns each node a `[0,1]` score based on cycle-cover constraints, then round
fractional values ≥ 0.5 into the kernel-graph. The LP round provides a principled,
approximation-theoretic inclusion decision beyond the heuristic score from T01.
The rounded LP solution may expand or contract the T01 selection; both the heuristic
and LP selections are reported.

## Build (checklist of concrete, code-level steps)

- [ ] `pub fn lp_round_kernel_graph(kernel_graph: &KernelGraph, lp_params: &LpRoundParams) -> Result<KernelGraph, CalyxError>` — constructs `LpProblem` via `mfvs_lp_problem` (PH31-T06), solves it (LP solver), rounds x_v ≥ 0.5 → included.
- [ ] `pub struct LpRoundParams { threshold: f64, fallback_to_heuristic: bool }` — `threshold` default 0.5; `fallback_to_heuristic`: if solver unavailable, emit `CALYX_KERNEL_LP_UNAVAILABLE` warning and return T01 heuristic result.
- [ ] After LP solve: `LpSolution.status == Optimal` required; `Infeasible` → `CALYX_KERNEL_LP_INFEASIBLE`; `NotSolved` → `CALYX_KERNEL_LP_UNAVAILABLE`.
- [ ] Report `lp_fraction` = (nodes rounded in by LP) / total; append to `KernelGraph` metadata.
- [ ] If LP result and heuristic result differ by > 20% of node count, log a warning
  (not an error) so the discrepancy is auditable.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: mock LP solution with values `[0.9, 0.3, 0.7, 0.1]` for a 4-node graph;
  threshold 0.5 → nodes 0 and 2 included; node count = 2.
- [ ] unit: `fallback_to_heuristic = true` with `SolveStatus::NotSolved` → function
  returns the T01 heuristic `KernelGraph` with a `CALYX_KERNEL_LP_UNAVAILABLE`
  warning in the log, no panic.
- [ ] unit: LP optimal on triangle graph `A→B→C→A` → at least one node included
  (the LP must round at least the MFVS minimum-cover node).
- [ ] edge: all LP values = exactly 0.5 → all nodes included (inclusive threshold).
- [ ] edge: all LP values = 0.0 → empty kernel-graph → `CALYX_KERNEL_EMPTY_RESULT`
  (a completely empty result is not allowed; must include at least 1 node).
- [ ] fail-closed: solver returns `Infeasible` → `CALYX_KERNEL_LP_INFEASIBLE`
  (not a silent empty result).

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `cargo test -p calyx-lodestar lp_round -- --nocapture` stdout.
- **Readback:** `cargo test -p calyx-lodestar lp_round 2>&1 | tee /tmp/ph32_t02_fsv.txt && cat /tmp/ph32_t02_fsv.txt`.
- **Prove:** mock LP unit test prints nodes 0 and 2 as selected; fallback test
  prints `CALYX_KERNEL_LP_UNAVAILABLE` warning; all tests pass; output attached to PH32 GitHub issue.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH32 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
