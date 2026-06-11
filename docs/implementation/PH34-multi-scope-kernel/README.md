# PH34 — Multi-scope kernel

**Stage:** S6 — Lodestar Kernel  ·  **Crate:** `calyx-lodestar`  ·
**PRD roadmap:** A21  ·  **Axioms:** A21, A10, A11

## Objective

Expose the kernel as a first-class parameterized operation over any data slice:
`build_kernel(scope, anchor?, params?)` for `AllAssociations`, `Collection`,
`Domain`, `Subgraph`, `TimeWindow`, `Tenant`, `Filter`, and `Union/Intersect`.
Each scope materializes its own subgraph, runs the full MFVS pipeline, and
reports its own measured kernel size, kernel-only recall, and grounded fraction —
never an assumed 1%. Scope results are cached by
`(scope_hash, panel_version, anchor_identity, corpus_identity)` for incremental
reuse (Anneal). Hierarchical kernel-of-regions handles huge scopes.
Per `08 §4b`: "calculate the kernel on many levels as a first-class, parameterized
operation — the same MFVS machinery, any slice of the data, any depth."

## Dependencies

- **Phases:** PH33 (kernel index + `kernel_answer` + recall test — all required;
  `build_kernel_pipeline` is the underlying engine), PH09 (Collection/Tenant/Anchor
  scoping metadata), PH24 (Subgraph scope uses search to bound the neighborhood)
- **Provides for:** PH43 (Anneal re-eval uses per-scope `IncrementalKernelEval`),
  PH48 (J objective uses per-scope kernel recall), PH72 (time-travel scope via
  `TimeWindow`)

## Current state (build off what exists)

`calyx-lodestar` has `build_kernel_pipeline`, `kernel_answer`, `grounding_gaps`,
and the recall test (PH33). This phase wraps them in the `Scope` enum and the
scope-cache layer. Hierarchical kernel-of-regions is new.
PH34 T01 (#233) is implemented and FSV-signed-off on aiwonder: `Scope`,
`AssocStore`, `scope_hash`, and `materialize_scope` cover all variants with
readbacks under `/home/croyse/calyx/data/fsv-issue233-scope-materialize-20260608`.
PH34 T02 (#234) is implemented and FSV-signed-off on aiwonder: `ScopeCache`,
`ScopeCacheKey`, and `CacheStats` cover bounded LRU reuse, panel-version
invalidation, hit/miss stats, zero-capacity behavior, and `u64::MAX` panel
versions with readbacks under
`/home/croyse/calyx/data/fsv-issue234-scope-cache-20260608`.
PH34 T03 (#235) is implemented and FSV-signed-off on aiwonder: scoped
`build_kernel`, `anchors_for_scope`, `ScopeKernelReport`, and
`report_all_scopes` cover cache dispatch, scoped anchors, per-scope report rows,
provisional ungrounded tagging, empty intersect reporting, panel-version cache
misses, and temporal fail-closed propagation with readbacks under
`/home/croyse/calyx/data/fsv-issue235-multi-scope-20260608`.
PH34 T04 (#236) is implemented and FSV-signed-off on aiwonder:
`RegionStore`, `RegionDescriptor`, `HierarchicalKernelParams`, and
`build_hierarchical_kernel` cover region-level kernels, deterministic
region-node IDs, scoped anchor projection, cache-backed drilldowns, direct
fallback when no regions exist, `all_members` union readback, single-region /
`max_regions = 1` / `drill_radius = 0` edge cases, and PH34 gate commands with
readbacks under `/home/croyse/calyx/data/fsv-issue236-hierarchical-20260608`.
PH34 T05 (#237) is implemented and FSV-signed-off on aiwonder:
`bridges`, `kernel_answer_scoped`, and `ScopeKernelReport.bridge_count` cover
frequency-sorted bridge nodes, disjoint/empty bridge results, scoped answer
paths that cannot leak outside materialized scope edges, scope-local answer
candidate ranking/index rows (#646), `AllAssociations` self-bridges, and a
union-scope MFVS readback proving the union kernel is not a naive member union.
Readbacks live under `/home/croyse/calyx/data/fsv-issue237-bridge-scopes-20260608`;
#646 scoped-candidate FSV lives under
`/home/croyse/calyx/data/fsv-issue646-scoped-answer-20260611T074856Z`.
PH34 T06 (#238) is implemented and FSV-signed-off on aiwonder: the real SciFact
corpus was measured at five scopes (`AllAssociations`, `Collection`,
`TimeWindow`, `Domain`, and `Union`), each wrote a distinct
`ScopeKernelReport`, recall gates passed, grounded fractions varied, bridge
nodes were non-empty, and the union diagnostic proved `mfvs_not_naive_union`.
Readbacks live under `/home/croyse/calyx/fsv/ph34_scope_*_20260608.json`.
PH34 T07 (#328) is implemented and FSV-signed-off on aiwonder: `ScopeCacheKey`
now includes anchor-set identity and corpus/store shard identity, with the real
`build_kernel` path proving same scope+panel+anchors hits and changed
anchor/corpus identity misses. Readbacks live under
`/home/croyse/calyx/data/fsv-issue328-scope-cache-identity-20260608`.

## Deliverables (file plan, each ≤500 lines)

| File | Responsibility |
|---|---|
| `crates/calyx-lodestar/src/scope.rs` | `Scope` enum (all 8 variants); `scope_hash(scope) -> [u8;32]`; `materialize_scope(scope, store) -> AssocGraph` |
| `crates/calyx-lodestar/src/scope_cache.rs` | `ScopeCache`: stores `(scope_hash, panel_version, anchor_identity, corpus_identity) -> Kernel`; LRU eviction; `cache_hit / cache_miss` counters |
| `crates/calyx-lodestar/src/multi_scope.rs` | `build_kernel(vault, scope, anchor_kind?, params?) -> Kernel`; dispatches through scope-cache; calls `build_kernel_pipeline` on miss |
| `crates/calyx-lodestar/src/hierarchical.rs` | `build_hierarchical_kernel(vault, scope, params) -> HierarchicalKernel`; kernel-of-regions first, then drill-down |
| `crates/calyx-lodestar/src/scope_report.rs` | `ScopeKernelReport { scope, kernel_size, kernel_only_recall, grounded_fraction, approx_factor }`; `report_all_scopes` |

## Tasks (atomic — all must pass for the phase to be DONE)

| Card | Title | Depends | Status |
|---|---|---|---|
| T01 | `Scope` enum + `materialize_scope` for all 8 variants | — (needs PH33) | Done / #233 |
| T02 | `ScopeCache`: identity-aware LRU cache | T01 | Done / #234 |
| T03 | `build_kernel(scope, ...)` dispatch + per-scope recall + grounded-fraction | T02 | Done / #235 |
| T04 | Hierarchical kernel-of-regions for huge scopes | T03 | Done / #236 |
| T05 | `Union`/`Intersect` composable scopes + bridge nodes | T04 | Done / #237 |
| T06 | FSV: >=4 distinct scopes on a real corpus, each with measured recall | T05 | Done / #238 |
| T07 | Scope-cache identity includes anchors and corpus/store shard | T03 | Done / #328 |

## FSV exit gate (the phase is DONE only when this is byte-proven on aiwonder)

1. `build_kernel` run at **≥4 distinct scopes** on a real corpus (e.g. one each
   of `AllAssociations`, `Collection(id)`, `TimeWindow(t0,t1)`, `Domain(anchor_kind)`).
2. Each scope produces a `ScopeKernelReport` with its own measured `kernel_only_recall`
   and `grounded_fraction` (values differ across scopes — not a constant).
3. Reports read back via `cat $CALYX_HOME/fsv/ph34_scope_*.json` on aiwonder.
4. Evidence (4 JSON files + printed summary table) attached to PH34 GitHub issue.
5. `Union(scope_a, scope_b)` produces a kernel that contains bridge nodes
   (constellations appearing in both sub-scopes); verified in the report.

## Risks / landmines

- **Scope materialization cost:** `AllAssociations` on a large corpus can be
  very slow; the scope cache and hierarchical kernel-of-regions are essential
  mitigations. Always run `AllAssociations` in a background task with a progress signal.
- **`scope_hash` stability:** must be deterministic across restarts; use a
  content-addressed hash of the scope enum variant + parameters (no timestamps
  in the hash); `panel_version` covers temporal changes.
- **`TimeWindow` requires the temporal lens (PH22/PH40):** if those aren't done,
  `TimeWindow` scope returns `CALYX_SCOPE_TEMPORAL_NOT_READY` (fail-closed).
- **`Union` kernel ≠ union of members:** the union kernel is the MFVS of the
  union graph, which may be smaller or larger than the union of individual kernels;
  never return `members_a ∪ members_b` as the union kernel without running MFVS.
