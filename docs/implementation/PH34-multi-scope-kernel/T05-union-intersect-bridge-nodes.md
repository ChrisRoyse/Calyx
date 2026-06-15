# PH34 · T05 — `Union`/`Intersect` composable scopes + bridge nodes

| Field | Value |
|---|---|
| **Phase** | PH34 — Multi-scope kernel |
| **Stage** | S6 — Lodestar Kernel |
| **Crate** | `calyx-lodestar` |
| **Files** | `crates/calyx-lodestar/src/multi_scope.rs` (<=500), `crates/calyx-lodestar/src/scope_report.rs` (<=500), `crates/calyx-lodestar/tests/ph34_bridge_tests.rs` (<=500) |
| **Depends on** | T03 (`build_kernel` dispatch), T01 (`materialize_scope` Union/Intersect) |
| **Axioms** | A21 |
| **PRD** | `dbprdplans/08 §4b` (`Union/Intersect(scopes)`, "kernel of A∩B, bridges between A and B"), `08 §5` (cross-domain bridge nodes) |

## Goal

Implement bridge-node detection for `Union` scopes: constellations that appear
in the kernel of both sub-scopes are "bridge nodes" that ground two domains at once
(per `08 §5`: "constellations that ground two domains at once — high value"). Expose
`bridges(scope_a, scope_b) -> Vec<CxId>` and `kernel_answer(scope)` routing through
bridge nodes when available. This completes the composable-answering model.

## Status

Implemented and FSV-signed-off on aiwonder on 2026-06-08 for #237. The
implementation adds `bridges`, `kernel_answer_scoped`, `bridge_count` in
`ScopeKernelReport`, and a call-site invariant comment making `Union` kernels
run MFVS over the materialized union graph instead of returning
`members_a ∪ members_b`.

## Build (checklist of concrete, code-level steps)

- [x] `pub fn bridges(store: &dyn AssocStore, scope_a: Scope, scope_b: Scope, anchor_kind: Option<AnchorKind>, params: KernelParams, cache: &mut ScopeCache) -> Result<Vec<CxId>>`:
  1. `kernel_a = build_kernel(store, scope_a, ...)`;
  2. `kernel_b = build_kernel(store, scope_b, ...)`;
  3. `bridges = kernel_a.members ∩ kernel_b.members` (intersection by `CxId`).
  4. Sort by descending frequency weight (A29: high-frequency bridge = highest value).
  5. Return sorted bridge list.
- [x] `pub fn kernel_answer_scoped(kernel_index: &KernelIndex, store: &dyn AssocStore, query_cx: CxId, query_vec: &[f32], scope: &Scope, anchored_kernel_nodes: &[CxId], max_hops: usize) -> Result<AnswerPath>` — wraps `kernel_answer` (PH33-T02) but first filters both `KernelIndex` rows and anchored candidates to the materialized scope's node set, then restricts traversal to scoped edges (#646).
- [x] Bridge nodes appear in `ScopeKernelReport` as a `bridge_count: usize` field (count of nodes
  appearing in multiple scope kernels) — added to the existing report struct from T03.
- [x] Empty bridge list (disjoint scopes) → return `vec![]` without error; no `CALYX_*` for empty bridges.
- [x] `Union` kernel is the MFVS of the union graph — NOT the union of individual members;
  add a comment `// IMPORTANT: Union kernel ≠ members_a ∪ members_b` at the call site.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: 2 collections with 2 shared high-centrality nodes; `bridges(coll_a, coll_b)` →
  both shared nodes in result; sorted by frequency weight descending.
- [x] unit: disjoint collections with no overlapping kernel members →
  `bridges(...)` = `[]`; no error.
- [x] unit: `kernel_answer_scoped` on a `Subgraph` scope → only traverses edges within
  the subgraph; does not leak into the full graph.
- [x] unit: `kernel_answer_scoped` with an out-of-scope high-score anchor and an
  in-scope lower-score reachable anchor → global top candidate is excluded from
  `scoped_index_rows`, and the in-scope anchor is selected (#646).
- [x] unit: `ScopeKernelReport.bridge_count` for the union scope = 2 (the 2 shared nodes).
- [x] edge: `bridges` with both scopes = `AllAssociations` → bridge list = all kernel members
  (every member is a bridge between A and A); length = kernel.members.len().
- [x] edge: `bridges` on two empty scopes → `[]`; no panic.
- [x] fail-closed: `kernel_answer_scoped` on a scope with no anchored kernel node →
  `CALYX_KERNEL_NO_ANCHORED_NODE` (not a silent empty answer).
- [x] fail-closed: if scope filtering removes all anchored candidates, return
  `CALYX_KERNEL_NO_ANCHORED_NODE` before answer selection (#646).

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** JSON readback files written by the PH34 T05 tests under
  `/home/croyse/calyx/data/fsv-issue237-bridge-scopes-20260608` on aiwonder.
- **Trigger:** `CALYX_FSV_ROOT=/home/croyse/calyx/data/fsv-issue237-bridge-scopes-20260608 cargo test -p calyx-lodestar --test ph34_bridge_tests -- --nocapture --test-threads=1`.
- **Readbacks and sha256:**
  - `bridges/ph34-bridges-shared-readback.json`:
    `c7ec97e8e1a5cff5c289339628a4c4ae0238445177dde7b206e7154c415894f2`
  - `empty/ph34-bridges-empty-readback.json`:
    `42a09941f378dbb5d6364d66177cf018b8521308b73f801521e8e454545e64c6`
  - `answer/ph34-scoped-answer-readback.json`:
    `dfc3735ea0abcef56356af841f46d5aad416d90cc2c0451f94c8f467182468a1`
  - `reports/ph34-bridge-report-readback.json`:
    `0a1e211e50006036096ea7f57dedc8ed9accef45877639481f458522dfb2f1ed`
  - `all/ph34-bridges-all-readback.json`:
    `567d3e8a9af9ca982ecafb57ad1df1605a43946eb1750b5411780d41cb504fbc`
  - `union/ph34-union-mfvs-readback.json`:
    `db4fc48b48b168b65aea45c54935ad8ff77fe260f6e4e90b506b6ace68b87f22`
  - FSV log:
    `e9d49626241504c7375b28932d616ebe1d7a305d3c3a76bb212dd830826227d5`
- **Prove:** shared bridges are two `CxId`s sorted by frequency, disjoint and
  empty scopes return `[]`, scoped answer refuses a path that only exists via an
  out-of-scope node, missing anchored nodes fail closed with
  `CALYX_KERNEL_NO_ANCHORED_NODE`, the union report has `bridge_count=2`,
  `AllAssociations` bridged with itself returns all kernel members, and a
  union-scope kernel is MFVS-derived (`mfvs_not_naive_union=true`).
- **#646 scoped-candidate FSV:** root
  `/home/croyse/calyx/data/fsv-issue646-scoped-answer-20260611T074856Z`;
  readback `fsv/ph34-scoped-answer-candidate-narrowing-readback.json`
  SHA-256 `34f467dc37ae2f66a352753269005c6b5694d993fed3e9c35e659c648a031bf3`.
  The readback proves global top candidate `09090909090909090909090909090909`
  was present in global index rows/hits, while `scoped_index_rows` and
  `scoped_anchors` contained only `01010101010101010101010101010101`; the
  selected answer anchor was `01010101010101010101010101010101` with one scoped
  hop to query `02020202020202020202020202020202`.

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) <= 500 lines (line-count gate green)
- [x] FSV evidence (readback output / screenshot) attached to the PH34 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
