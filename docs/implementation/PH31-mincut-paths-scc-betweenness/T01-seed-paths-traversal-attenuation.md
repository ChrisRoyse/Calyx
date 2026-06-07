# PH31 · T01 — Seed + adapt calyx-paths traversal + hop-attenuation

| Field | Value |
|---|---|
| **Phase** | PH31 — mincut/paths: graph build + SCC + betweenness |
| **Stage** | S6 — Lodestar Kernel |
| **Crate** | `calyx-paths` |
| **Files** | `crates/calyx-paths/src/lib.rs` (≤500), `crates/calyx-paths/src/traversal.rs` (≤500), `crates/calyx-paths/src/attenuation.rs` (≤500) |
| **Depends on** | — (first card; seeds from ContextGraph copy) |
| **Axioms** | A29, `19 §6` |
| **PRD** | `dbprdplans/08 §2`, `dbprdplans/08 §4.2` |

## Goal

Copy the ContextGraph `context-graph-paths` source into `crates/calyx-paths/src/`
as a seed (never link the live project, per `19 §6`), then adapt it to Calyx's
`CxId`-keyed graph and implement bidirectional BFS/DFS traversal with the
`0.9^hop` hop-attenuation that the kernel-answer path requires (`08 §4.2`).

## Build (checklist of concrete, code-level steps)

- [ ] Copy ContextGraph paths source files into `crates/calyx-paths/src/`; rename
  `NodeId` → `CxId` (from `calyx-core`); update `Cargo.toml` deps accordingly.
- [ ] `pub fn reach(graph: &AssocGraph, src: CxId, dst: CxId, max_hops: usize) -> Option<Vec<CxId>>`
  — bidirectional BFS meeting-in-the-middle; returns the shortest hop path.
- [ ] `pub fn reach_scored(graph: &AssocGraph, src: CxId, max_hops: usize) -> Vec<(CxId, f32)>`
  — BFS from `src`; every reachable node gets score `edge_weight * 0.9_f32.powi(hop)`.
- [ ] `pub fn attenuate(base_score: f32, hops: u32) -> f32` = `base_score * 0.9_f32.powi(hops as i32)`;
  inverse: `pub fn deattenuate(attenuated: f32, hops: u32) -> f32`.
- [ ] All traversal functions accept a `max_hops: usize` bound; exceeding → `CALYX_PATHS_MAX_HOPS`.
- [ ] `lib.rs` re-exports `graph`, `traversal`, `attenuation` modules; `#![deny(warnings)]`.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: linear chain `A→B→C→D`; `reach(A, D, 3)` = `[A,B,C,D]`;
  `reach_scored(A, 3)` gives `B=0.9`, `C=0.81`, `D=0.729` for unit-weight edges.
- [ ] unit: `attenuate(1.0, 0)` = `1.0`; `attenuate(1.0, 1)` = `0.9`;
  `attenuate(1.0, 10)` ≈ `0.34868`; `deattenuate(attenuate(x,k), k)` = `x` within ε=1e-6.
- [ ] proptest: `reach_scored` scores are strictly monotone-decreasing with hops
  for a uniform-weight chain of length `n` in `1..20`.
- [ ] edge: `reach(A, B, 0)` where A≠B → `None`; `reach(A, A, 0)` → `Some([A])`.
- [ ] edge: disconnected graph → `reach(A, Z, 100)` = `None` (not an empty vec).
- [ ] edge: `max_hops` exactly met (path length == max_hops) returns path;
  length = max_hops+1 returns `Err(CALYX_PATHS_MAX_HOPS)`.
- [ ] fail-closed: zero-node graph → `reach` on any ids → `CALYX_PATHS_NODE_NOT_FOUND`.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** stdout of `cargo test -p calyx-paths -- --nocapture` on aiwonder.
- **Readback:** run `cargo test -p calyx-paths 2>&1 | tee /tmp/ph31_t01_fsv.txt`
  then `cat /tmp/ph31_t01_fsv.txt`.
- **Prove:** all unit + proptest + edge tests pass (0 failures); attenuation
  values printed match `0.9^k` to ε=1e-6; no test silently skipped.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH31 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
