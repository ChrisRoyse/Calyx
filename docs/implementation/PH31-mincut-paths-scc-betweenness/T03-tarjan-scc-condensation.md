# PH31 · T03 — Tarjan SCC condensation

> **STATUS: ✅ DONE / FSV-signed-off.** Implemented in
> `crates/calyx-mincut/src/scc.rs` with Tarjan SCC partitioning, deterministic
> component membership, condensation edges, DAG check, and graph-mismatch
> fail-closed validation. aiwonder FSV readback: `ph31-scc-readback.json`.

> Historical checklist note: the unchecked implementation prompts below were
> satisfied by the closed Stage 6 evidence; current state is the status/evidence
> block above.

| Field | Value |
|---|---|
| **Phase** | PH31 — mincut/paths: graph build + SCC + betweenness |
| **Stage** | S6 — Lodestar Kernel |
| **Crate** | `calyx-mincut` |
| **Files** | `crates/calyx-mincut/src/lib.rs` (≤500), `crates/calyx-mincut/src/scc.rs` (≤500) |
| **Depends on** | T02 (`AssocGraph` in calyx-paths) |
| **Axioms** | A29, `19 §6` |
| **PRD** | `dbprdplans/08 §3` (Stage 1: Condense) |

## Goal

Implement Tarjan's SCC algorithm on `AssocGraph`; produce the SCC partition
(a list of strongly-connected components) and the condensation DAG (one node per
SCC). This is MFVS pipeline Stage 1: "collapse strongly-connected blobs; acyclic
part is already groundable" (`08 §3`). Seed from ContextGraph `context-graph-mincut`
source (copy into `crates/calyx-mincut/src/`, never link).

## Build (checklist of concrete, code-level steps)

- [ ] Copy ContextGraph mincut source into `crates/calyx-mincut/src/`; rename
  types to use `CxId` and `AssocGraph`; update `Cargo.toml`.
- [ ] `pub fn tarjan_scc(graph: &AssocGraph) -> SccResult` where
  `SccResult { components: Vec<Vec<CxId>>, component_of: HashMap<CxId, usize> }`.
  Components ordered in reverse topological order of the condensation DAG.
- [ ] `pub fn condensate(graph: &AssocGraph, scc: &SccResult) -> CondensedGraph`
  where `CondensedGraph` is an `AssocGraph` over `SccId` (usize) nodes with
  aggregate weights (max of member edge weights); self-loops in the condensed
  graph are illegal (if a SCC has internal edges, they collapse into the node).
- [ ] Singleton SCCs (no internal edges) pass through unchanged; trivially acyclic
  part of the condensate is verified by checking the condensate is a DAG.
- [ ] `lib.rs` re-exports `scc`, `betweenness`, `lp_scaffold`, `graph_builder`; `#![deny(warnings)]`.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: planted graph — three nodes in a cycle `A→B→C→A`, plus `D→A`;
  expected SCCs: `{A,B,C}` as one component, `{D}` as another;
  condensate has 2 nodes, 1 edge `{D}→{A,B,C}`.
- [ ] unit: pure DAG (no cycles) → each node is its own SCC; condensate isomorphic
  to original.
- [ ] unit: two disjoint cycles `A→B→A` and `C→D→C` connected by `A→C` →
  3 SCCs: `{A,B}`, `{C,D}`, and the condensate has edge `{A,B}→{C,D}`.
- [ ] proptest: for any random DAG (no cycles), `tarjan_scc` returns `n` singleton
  SCCs where `n` = node count.
- [ ] edge: single-node graph → 1 SCC = `[node]`; condensate = same 1 node, 0 edges.
- [ ] edge: fully-connected clique of 5 nodes → 1 SCC containing all 5.
- [ ] fail-closed: `condensate` called with a `SccResult` from a different graph
  (node count mismatch) → `CALYX_SCC_GRAPH_MISMATCH`.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `cargo test -p calyx-mincut scc -- --nocapture` stdout on aiwonder.
- **Readback:** `cargo test -p calyx-mincut scc 2>&1 | tee /tmp/ph31_t03_fsv.txt && cat /tmp/ph31_t03_fsv.txt`.
- **Prove:** planted-graph test prints the exact SCC partition `{A,B,C}, {D}`
  and condensation edge `{D}→{ABC}`; proptest passes all iterations;
  output attached to PH31 GitHub issue confirms computed == known.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH31 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
