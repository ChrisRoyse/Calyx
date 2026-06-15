# PH32 · T04 — Tournament 2-approx + bounded-genus O(g) specializations

> **STATUS: ✅ DONE / FSV-signed-off.** Implemented in
> `crates/calyx-lodestar/src/dfvs.rs` with tournament detection,
> `Tournament2Approx`, genus estimate, `BoundedGenus`, automatic dispatch,
> genus-too-large fail-closed behavior, and #645 honest-bound reporting. aiwonder
> FSV readback: `ph32-specialized-dfvs-readback.json`; #645 adds exact vs
> approximate tau readback under
> `/home/croyse/calyx/data/fsv-issue645-dfvs-honest-20260611T072428Z`.

> Historical checklist note: the unchecked implementation prompts below were
> satisfied by the closed Stage 6 evidence; current state is the status/evidence
> block above.

| Field | Value |
|---|---|
| **Phase** | PH32 — Kernel-graph (~10% target) + directed MFVS (~1% target) |
| **Stage** | S6 — Lodestar Kernel |
| **Crate** | `calyx-lodestar` |
| **Files** | `crates/calyx-lodestar/src/dfvs.rs` (≤500) |
| **Depends on** | T03 (`DfvsResult`, `DfvsMethod` enum, `dfvs_approx`) |
| **Axioms** | A10 |
| **PRD** | `dbprdplans/08 §3` (Stage 3: tournament 2-approx; bounded-genus `O(g)`-approx) |

## Goal

Add the two specialised DFVS approximation algorithms: (1) 2-approximation for
near-tournament graphs (graphs where every pair of nodes has at least one directed
edge — common in densely-associated corpus regions); (2) `O(g)`-approximation for
bounded-genus subgraphs (planar or near-planar sub-regions). Both are dispatched
automatically when `dfvs_approx` detects the graph satisfies the structural condition.
The `DfvsMethod` variant is set accordingly. `approx_factor` is exact `1.0`
only when exact search or a tight lower-bound certificate proves it; otherwise
it reports the conservative observed `|members| / tau_star_estimate` bound and
never clamps that value downward.

## Build (checklist of concrete, code-level steps)

- [ ] `pub fn is_tournament(graph: &AssocGraph) -> bool` — returns true iff for
  every pair `(u, v)` with `u ≠ v`, at least one of `u→v` or `v→u` exists.
- [ ] `pub fn tournament_2approx(graph: &AssocGraph) -> DfvsResult` — implements
  the 2-approximation for directed FVS in tournaments (see arXiv:1809.08437):
  repeatedly remove the node with max out-degree in the remaining tournament until
  acyclic; `approx_factor ≤ 2.0`; `method = Tournament2Approx`.
- [ ] `pub fn genus_estimate(graph: &AssocGraph) -> usize` — estimate the graph's
  genus via Euler characteristic approximation; return `0` if planar estimate.
- [ ] `pub fn bounded_genus_approx(graph: &AssocGraph, genus: usize) -> DfvsResult` —
  `O(g)`-approximation via face-enumeration on the embedded graph; `method = BoundedGenus`;
  `approx_factor ≤ genus + 1` (or a tighter constant derived from the embedding).
- [ ] `dfvs_approx` dispatch: if `is_tournament` -> call `tournament_2approx`;
  else if `genus_estimate <= 2` -> call `bounded_genus_approx`; else -> exact/greedy local search.
- [ ] All three methods set `approx_factor` from the honest certificate: exact
  or tight lower-bound proof -> `1.0`; otherwise the conservative observed
  `|members| / tau_star_estimate`, not a downward-clamped theoretical label.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: 4-node tournament (complete directed graph with tie-breaking); `is_tournament` = true;
  `tournament_2approx` returns a valid FVS (removing members → acyclic).
- [ ] unit: `approx_factor ≤ 2.0` for the tournament test; method = `Tournament2Approx`.
- [ ] unit: planar graph (K4 with one edge removed); `genus_estimate` = 0;
  `bounded_genus_approx(g, 0)` returns a valid FVS.
- [ ] unit: `dfvs_approx` on a tournament → automatically dispatches to
  `tournament_2approx` (method field = `Tournament2Approx` in result).
- [ ] proptest: for any random tournament graph, removing `tournament_2approx.members`
  yields a DAG.
- [ ] edge: 2-node graph `A→B` and `B→A` (minimal tournament); FVS = 1 node;
  `approx_factor ≤ 2.0`.
- [ ] fail-closed: `bounded_genus_approx` called with `genus > 100` →
  `CALYX_DFVS_GENUS_TOO_LARGE` (fall back to LP path instead of hanging).

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `cargo test -p calyx-lodestar tournament -- --nocapture` and
  `cargo test -p calyx-lodestar genus -- --nocapture` stdout.
- **Readback:** `cargo test -p calyx-lodestar dfvs 2>&1 | tee /tmp/ph32_t04_fsv.txt && cat /tmp/ph32_t04_fsv.txt`.
- **Prove:** tournament test prints `method=Tournament2Approx`; genus test
  prints `method=BoundedGenus`; #645 readback shows exact and approximate tau
  evidence are distinguishable; proptest passes confirming acyclicity; output
  attached to PH32 GitHub issue.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH32 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
