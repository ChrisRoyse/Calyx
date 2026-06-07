# PH34 · T05 — `Union`/`Intersect` composable scopes + bridge nodes

| Field | Value |
|---|---|
| **Phase** | PH34 — Multi-scope kernel |
| **Stage** | S6 — Lodestar Kernel |
| **Crate** | `calyx-lodestar` |
| **Files** | `crates/calyx-lodestar/src/multi_scope.rs` (≤500) |
| **Depends on** | T03 (`build_kernel` dispatch), T01 (`materialize_scope` Union/Intersect) |
| **Axioms** | A21 |
| **PRD** | `dbprdplans/08 §4b` (`Union/Intersect(scopes)`, "kernel of A∩B, bridges between A and B"), `08 §5` (cross-domain bridge nodes) |

## Goal

Implement bridge-node detection for `Union` scopes: constellations that appear
in the kernel of both sub-scopes are "bridge nodes" that ground two domains at once
(per `08 §5`: "constellations that ground two domains at once — high value"). Expose
`bridges(scope_a, scope_b) -> Vec<CxId>` and `kernel_answer(scope)` routing through
bridge nodes when available. This completes the composable-answering model.

## Build (checklist of concrete, code-level steps)

- [ ] `pub fn bridges(vault: &mut Vault, scope_a: Scope, scope_b: Scope, anchor_kind: Option<AnchorKind>, cache: &mut ScopeCache) -> Result<Vec<CxId>, CalyxError>`:
  1. `kernel_a = build_kernel(vault, scope_a, ...)`;
  2. `kernel_b = build_kernel(vault, scope_b, ...)`;
  3. `bridges = kernel_a.members ∩ kernel_b.members` (intersection by `CxId`).
  4. Sort by descending frequency weight (A29: high-frequency bridge = highest value).
  5. Return sorted bridge list.
- [ ] `pub fn kernel_answer_scoped(kernel_index: &KernelIndex, graph: &AssocGraph, query_cx: CxId, scope: &Scope, anchor_kind: Option<AnchorKind>, max_hops: usize) -> Result<AnswerPath, CalyxError>` — wraps `kernel_answer` (PH33-T02) but restricts traversal to edges within the materialized scope's node set.
- [ ] Bridge nodes appear in `ScopeKernelReport` as a `bridge_count: usize` field (count of nodes
  appearing in multiple scope kernels) — added to the existing report struct from T03.
- [ ] Empty bridge list (disjoint scopes) → return `vec![]` without error; no `CALYX_*` for empty bridges.
- [ ] `Union` kernel is the MFVS of the union graph — NOT the union of individual members;
  add a comment `// IMPORTANT: Union kernel ≠ members_a ∪ members_b` at the call site.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: 2 collections with 2 shared high-centrality nodes; `bridges(coll_a, coll_b)` →
  both shared nodes in result; sorted by frequency weight descending.
- [ ] unit: disjoint collections with no overlapping kernel members →
  `bridges(...)` = `[]`; no error.
- [ ] unit: `kernel_answer_scoped` on a `Subgraph` scope → only traverses edges within
  the subgraph; does not leak into the full graph.
- [ ] unit: `ScopeKernelReport.bridge_count` for the union scope = 2 (the 2 shared nodes).
- [ ] edge: `bridges` with both scopes = `AllAssociations` → bridge list = all kernel members
  (every member is a bridge between A and A); length = kernel.members.len().
- [ ] edge: `bridges` on two empty scopes → `[]`; no panic.
- [ ] fail-closed: `kernel_answer_scoped` on a scope with no anchored kernel node →
  `CALYX_KERNEL_NO_ANCHORED_NODE` (not a silent empty answer).

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `cargo test -p calyx-lodestar bridges -- --nocapture` stdout.
- **Readback:** `cargo test -p calyx-lodestar multi_scope 2>&1 | tee /tmp/ph34_t05_fsv.txt && cat /tmp/ph34_t05_fsv.txt`.
- **Prove:** shared-node test prints 2 bridge `CxId`s sorted by frequency;
  disjoint test prints empty `[]`; union-kernel test confirms `Union` kernel
  came from MFVS not naive union; output attached to PH34 GitHub issue.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH34 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
