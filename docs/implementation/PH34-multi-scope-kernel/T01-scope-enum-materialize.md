# PH34 · T01 — `Scope` enum + `materialize_scope` for all 8 variants

| Field | Value |
|---|---|
| **Phase** | PH34 — Multi-scope kernel |
| **Stage** | S6 — Lodestar Kernel |
| **Crate** | `calyx-lodestar` |
| **Files** | `crates/calyx-lodestar/src/scope.rs` (≤500) |
| **Depends on** | PH33-T01 (kernel index + pipeline), PH09 (CxId, Collection, Anchor types) |
| **Axioms** | A21 |
| **PRD** | `dbprdplans/08 §4b` |

## Status

DONE / FSV-signed-off on aiwonder (2026-06-08). Implemented in
`crates/calyx-lodestar/src/scope.rs` with public `Scope`, `AssocStore`,
`scope_hash`, and `materialize_scope`. Later Vault/Aster code can implement
`AssocStore` for real metadata without changing PH34 scope semantics.

Evidence root:
`/home/croyse/calyx/data/fsv-issue233-scope-materialize-20260608`.

| Artifact | SHA-256 |
|---|---|
| `hash/ph34-scope-hash-readback.json` | `67152bce474f9c7f00b462ba8e535589a0ff5052b4b983c063a70c85fe935de6` |
| `counts/ph34-scope-counts-readback.json` | `3607b63d75535c2fe672a4565b63d51192196ddceda31b625b155fd57d43a3bb` |
| `variants/ph34-scope-variants-readback.json` | `502cf6858bfb23178d939c2a38bba37e2ad5751e737c7664a5646d86d9e79810` |
| `edges/ph34-scope-edges-readback.json` | `160964d6eb03c2d6d8f9edcf560c6b5f0989217262777763356c59e1ea0c11ad` |
| `ph34_t01_fsv.log` | `f63825ae69652dd5e6dbe29b03ac8b2063ce035e6a19edbb0e1428bdda779327` |

Key readbacks: collection scope = `4` nodes; union = `6`; intersect = `1`;
`scope_hash(AllAssociations)` =
`9bcc9eef3da72eaed03ea54c2b0086368d119cf274516e1fb6706aaf487fe7d5`;
all remaining variants materialize (`all=10`, `domain=10`, `subgraph=3`,
`time_window=3`, `tenant=2`, `filter=5`); fail-closed edges produce
`CALYX_COLLECTION_NOT_FOUND`, `CALYX_SCOPE_TEMPORAL_NOT_READY`,
`CALYX_SCOPE_DEPTH_EXCEEDED`, and `CALYX_SCOPE_TENANT_NOT_FOUND`.

## Goal

Define the `Scope` enum with all 8 variants (`AllAssociations`, `Collection`,
`Domain`, `Subgraph`, `TimeWindow`, `Tenant`, `Filter`, `Union`/`Intersect`) and
implement `materialize_scope(scope, store) -> AssocGraph` which converts each
scope into the subgraph of the full `AssocGraph` that the MFVS pipeline will
process. Also implement `scope_hash(scope) -> [u8;32]` for cache keying.

## Build (checklist of concrete, code-level steps)

- [x] `pub enum Scope { AllAssociations, Collection(CollectionId), Domain(AnchorKind), Subgraph { query: CxId, radius: usize }, TimeWindow { t0: Timestamp, t1: Timestamp }, Tenant(TenantId), Filter(FilterExpr), Union(Box<Scope>, Box<Scope>), Intersect(Box<Scope>, Box<Scope>) }`.
- [x] `pub fn scope_hash(scope: &Scope) -> [u8; 32]` — deterministic Blake3 hash of
  the serialized scope; stable across restarts; `panel_version` is NOT included
  here (it is the cache key's second component).
- [x] `pub fn materialize_scope(scope: &Scope, store: &dyn AssocStore) -> Result<AssocGraph, CalyxError>`:
  - `AllAssociations` → full graph from store.
  - `Collection(id)` → nodes belonging to collection `id`; edges between them.
  - `Domain(anchor_kind)` → nodes reachable from any anchor of `anchor_kind`.
  - `Subgraph { query, radius }` → BFS neighborhood of `query` within `radius` hops.
  - `TimeWindow { t0, t1 }` → nodes created/updated in `[t0, t1]`; if temporal
    lens not ready → `CALYX_SCOPE_TEMPORAL_NOT_READY`.
  - `Tenant(id)` → nodes belonging to tenant `id`.
  - `Filter(expr)` → nodes matching `expr` (scalar/metadata predicate).
  - `Union(a, b)` → `materialize_scope(a) ∪ materialize_scope(b)` (merge edges).
  - `Intersect(a, b)` → `materialize_scope(a) ∩ materialize_scope(b)` (keep only
    nodes in both; edges between them).
- [x] `Union` and `Intersect` are recursive (depth-limited to 5 levels; deeper →
  `CALYX_SCOPE_DEPTH_EXCEEDED`).
- [x] All variants that produce empty graphs return `AssocGraph::empty()` without error.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: `scope_hash(AllAssociations)` is a fixed 32-byte value (embed as a
  const in the test); same call twice returns the same hash.
- [x] unit: `materialize_scope(Collection(id1))` on a 10-node store where 4 belong
  to `id1` → subgraph with exactly 4 nodes.
- [x] unit: `materialize_scope(Union(Collection(id1), Collection(id2)))` where id1
  has 4 nodes, id2 has 3 nodes, 1 overlapping → subgraph with 6 nodes.
- [x] unit: `materialize_scope(Intersect(Collection(id1), Collection(id2)))` with
  1 overlapping node → subgraph with 1 node.
- [x] unit: `Subgraph { query: A, radius: 2 }` on a chain `A→B→C→D` →
  subgraph = `{A, B, C}` (nodes within 2 hops).
- [x] edge: `TimeWindow` with temporal lens not initialized →
  `CALYX_SCOPE_TEMPORAL_NOT_READY`.
- [x] edge: `Union` nested 6 levels deep → `CALYX_SCOPE_DEPTH_EXCEEDED`.
- [x] fail-closed: `Collection` with unknown `CollectionId` → `CALYX_COLLECTION_NOT_FOUND`.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** JSON readbacks under
  `/home/croyse/calyx/data/fsv-issue233-scope-materialize-20260608` and test stdout.
- **Readback:** `CALYX_FSV_ROOT=/home/croyse/calyx/data/fsv-issue233-scope-materialize-20260608 cargo test -p calyx-lodestar --test ph34_scope_tests -- --nocapture 2>&1 | tee /home/croyse/calyx/data/fsv-issue233-scope-materialize-20260608/ph34_t01_fsv.log`.
- **Prove:** collection-scope test prints node count `4`; union test prints `6`;
  intersect test prints `1`; `scope_hash` test prints the fixed 32-byte hex and
  confirms stability; all tests pass; output attached to PH34 GitHub issue.

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] FSV evidence (readback output / screenshot) attached to the PH34 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
