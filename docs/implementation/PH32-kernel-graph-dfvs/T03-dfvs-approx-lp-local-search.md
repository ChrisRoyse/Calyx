# PH32 · T03 — MFVS LP-relaxation approx + local search (`dfvs_approx`)

| Field | Value |
|---|---|
| **Phase** | PH32 — Kernel-graph (~10%) + directed MFVS (~1%) |
| **Stage** | S6 — Lodestar Kernel |
| **Crate** | `calyx-lodestar` |
| **Files** | `crates/calyx-lodestar/src/dfvs.rs` (≤500) |
| **Depends on** | T02 (kernel-graph as input), PH31-T06 (LP scaffold) |
| **Axioms** | A10 |
| **PRD** | `dbprdplans/08 §3` (Stage 3: LP-relaxation `O(log τ* log log τ*)` approx + local search) |

## Goal

Implement `dfvs_approx`: the main directed MFVS approximation on the kernel-graph
(`KernelGraph` from T02). Uses LP-relaxation rounding (`O(log τ* log log τ*)`)
followed by local-search improvement. The approximation factor τ_actual/τ* is
computed and stored in `DfvsResult.approx_factor` — always reported, never asserted.
Seeded from ContextGraph `context-graph-solver` source (copied, never linked).

## Build (checklist of concrete, code-level steps)

- [ ] `pub struct DfvsResult { members: Vec<CxId>, approx_factor: f64, tau_star_estimate: usize, method: DfvsMethod }` where `DfvsMethod` is `LpLocalSearch | Tournament2Approx | BoundedGenus`.
- [ ] `pub fn dfvs_approx(graph: &KernelGraph) -> Result<DfvsResult, CalyxError>`:
  - Step 1: solve LP relaxation on kernel-graph; round x_v ≥ 0.5 into FVS candidate set.
  - Step 2: local-search — for each node in candidate: remove it; if graph is still
    acyclic (no directed cycle covers it), remove it from FVS (greedy shrink).
  - Step 3: verify result is indeed an FVS (removing members makes the graph acyclic);
    if not → `CALYX_DFVS_VERIFICATION_FAILED`.
- [ ] `approx_factor`: compute as `|members| / tau_star_estimate` where
  `tau_star_estimate` = LP relaxation objective value (lower bound on τ*);
  log `O(log τ* log log τ*)` bound for reference.
- [ ] Seeds from ContextGraph solver: copy source, rename to Calyx types, document
  the seed commit hash in a `// SEED:` comment at file top.
- [ ] Empty kernel-graph → `DfvsResult { members: vec![], approx_factor: 1.0, tau_star_estimate: 0, method: LpLocalSearch }`.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: triangle `A→B→C→A`; `dfvs_approx` must return exactly 1 member
  (any one of A, B, or C is a valid FVS); `approx_factor ≤ 3.0` (worst case pick any).
- [ ] unit: planted FVS — graph with a known planted MFVS of 2 nodes (inserted as
  high-centrality + cycle-cover nodes); `members` contains both planted nodes.
- [ ] unit: acyclic DAG (`A→B→C`) → `members = []`; `approx_factor = 1.0`.
- [ ] unit: after removing `members` from the triangle graph, result is acyclic
  (verify by `tarjan_scc` — all singleton SCCs).
- [ ] proptest: for any random graph, removing `dfvs_approx.members` from the graph
  results in a DAG (no directed cycles).
- [ ] edge: single self-loop `A→A` → `members = [A]`; removing A yields empty DAG.
- [ ] fail-closed: LP returns `Infeasible` → `CALYX_DFVS_LP_INFEASIBLE` (not a
  silent empty result).

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `cargo test -p calyx-lodestar dfvs -- --nocapture` stdout.
- **Readback:** `cargo test -p calyx-lodestar dfvs 2>&1 | tee /tmp/ph32_t03_fsv.txt && cat /tmp/ph32_t03_fsv.txt`.
- **Prove:** triangle test prints 1 member and `approx_factor ≤ 3.0`;
  planted-FVS test prints both planted node IDs in `members`;
  proptest passes all iterations confirming acyclicity post-removal;
  output attached to PH32 GitHub issue.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH32 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
