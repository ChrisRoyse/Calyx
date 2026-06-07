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

## Goal

Define the `Scope` enum with all 8 variants (`AllAssociations`, `Collection`,
`Domain`, `Subgraph`, `TimeWindow`, `Tenant`, `Filter`, `Union`/`Intersect`) and
implement `materialize_scope(scope, store) -> AssocGraph` which converts each
scope into the subgraph of the full `AssocGraph` that the MFVS pipeline will
process. Also implement `scope_hash(scope) -> [u8;32]` for cache keying.

## Build (checklist of concrete, code-level steps)

- [ ] `pub enum Scope { AllAssociations, Collection(CollectionId), Domain(AnchorKind), Subgraph { query: CxId, radius: usize }, TimeWindow { t0: Timestamp, t1: Timestamp }, Tenant(TenantId), Filter(FilterExpr), Union(Box<Scope>, Box<Scope>), Intersect(Box<Scope>, Box<Scope>) }`.
- [ ] `pub fn scope_hash(scope: &Scope) -> [u8; 32]` — deterministic Blake3 hash of
  the serialized scope; stable across restarts; `panel_version` is NOT included
  here (it is the cache key's second component).
- [ ] `pub fn materialize_scope(scope: &Scope, store: &dyn AssocStore) -> Result<AssocGraph, CalyxError>`:
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
- [ ] `Union` and `Intersect` are recursive (depth-limited to 5 levels; deeper →
  `CALYX_SCOPE_DEPTH_EXCEEDED`).
- [ ] All variants that produce empty graphs return `AssocGraph::empty()` without error.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: `scope_hash(AllAssociations)` is a fixed 32-byte value (embed as a
  const in the test); same call twice returns the same hash.
- [ ] unit: `materialize_scope(Collection(id1))` on a 10-node store where 4 belong
  to `id1` → subgraph with exactly 4 nodes.
- [ ] unit: `materialize_scope(Union(Collection(id1), Collection(id2)))` where id1
  has 4 nodes, id2 has 3 nodes, 1 overlapping → subgraph with 6 nodes.
- [ ] unit: `materialize_scope(Intersect(Collection(id1), Collection(id2)))` with
  1 overlapping node → subgraph with 1 node.
- [ ] unit: `Subgraph { query: A, radius: 2 }` on a chain `A→B→C→D` →
  subgraph = `{A, B, C}` (nodes within 2 hops).
- [ ] edge: `TimeWindow` with temporal lens not initialized →
  `CALYX_SCOPE_TEMPORAL_NOT_READY`.
- [ ] edge: `Union` nested 6 levels deep → `CALYX_SCOPE_DEPTH_EXCEEDED`.
- [ ] fail-closed: `Collection` with unknown `CollectionId` → `CALYX_COLLECTION_NOT_FOUND`.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `cargo test -p calyx-lodestar scope -- --nocapture` stdout.
- **Readback:** `cargo test -p calyx-lodestar scope 2>&1 | tee /tmp/ph34_t01_fsv.txt && cat /tmp/ph34_t01_fsv.txt`.
- **Prove:** collection-scope test prints node count `4`; union test prints `6`;
  intersect test prints `1`; `scope_hash` test prints the fixed 32-byte hex and
  confirms stability; all tests pass; output attached to PH34 GitHub issue.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH34 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
