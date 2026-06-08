# PH34 T03 - `build_kernel(scope, ...)` dispatch + per-scope reports

| Field | Value |
|---|---|
| **Phase** | PH34 - Multi-scope kernel |
| **Stage** | S6 - Lodestar Kernel |
| **Crate** | `calyx-lodestar` |
| **Files** | `crates/calyx-lodestar/src/multi_scope.rs` (<=500), `crates/calyx-lodestar/src/scope_report.rs` (<=500) |
| **Depends on** | T01 (`materialize_scope`), T02 (`ScopeCache`) |
| **Axioms** | A21, A10 |
| **PRD** | `dbprdplans/08 section 4b`, `08 section 8` |

## Goal

Implement the top-level `build_kernel(vault, scope, anchor_kind?, params?) ->
Kernel` operation. It materializes the requested scope, resolves scoped anchors,
dispatches through the identity-aware cache, and runs `build_kernel_pipeline` on
a miss. Each scope carries measured recall and groundedness; `ScopeKernelReport`
aggregates those values without recomputing.

## Status

Implemented in issue #235 and hardened in #328. Base aiwonder FSV readbacks live
under `/home/croyse/calyx/data/fsv-issue235-multi-scope-20260608`; identity
readbacks live under
`/home/croyse/calyx/data/fsv-issue328-scope-cache-identity-20260608`.

## Build

- [x] `build_kernel(store, scope, anchor_kind, params, cache) -> Result<Kernel>`:
  1. `materialize_scope(&scope, store)` -> `subgraph`.
  2. Resolve scoped anchor kinds and anchor IDs from the materialized subgraph.
  3. Compute `ScopeCacheKey::new(scope_hash(&scope), params.panel_version,
     anchor_identity, params.corpus_shard_hash)`.
  4. Cache hit -> return cloned `Kernel`.
  5. Cache miss -> `build_kernel_pipeline(&subgraph, &anchors, &params)`;
     `cache.insert(key, kernel.clone())`; return `kernel`.
- [x] `anchors_for_scope(scope, store, anchor_kind)` selects only anchors present
  in the materialized scope.
- [x] `ScopeKernelReport` records scope hash, kernel size, kernel graph size,
  kernel-only recall, grounded fraction, approx factor, and bridge count.
- [x] `report_all_scopes(kernels)` collects one row per supplied scope/kernel
  pair from the already-built kernel fields.
- [x] Ungrounded scope kernels are tagged with `CALYX_KERNEL_UNGROUNDED` and
  `trust=provisional`.

## Tests

- [x] unit: `AllAssociations` and `Collection(id1)` on a known cyclic store both
  produce non-empty kernels, and the collection kernel is a subset of all.
- [x] unit: same scope, panel, anchors, and corpus hits the cache on the second
  call.
- [x] unit: same scope/panel with alternate anchors misses and creates a second
  cache entry.
- [x] unit: same scope/panel/anchors with a changed corpus hash misses and
  creates a third cache entry.
- [x] unit: `report_all_scopes` emits 3 rows and sizes match kernel members.
- [x] edge: unanchored scope is provisional.
- [x] edge: panel version bump misses cache.
- [x] edge: empty `Intersect` reports zero members.
- [x] fail-closed: temporal-not-ready propagates and does not populate cache.

## FSV

- **Base trigger:** `CALYX_FSV_ROOT=/home/croyse/calyx/data/fsv-issue235-multi-scope-20260608 cargo test -p calyx-lodestar --test ph34_multi_scope_tests -- --nocapture --test-threads=1`
- **Base readbacks:**
  - `subset/ph34-multi-scope-subset-readback.json`: collection members are a
    subset of all-association members.
  - `cache/ph34-multi-scope-cache-readback.json`: `hits=1`, `misses=1`,
    `current_size=1`.
  - `reports/ph34-multi-scope-reports-readback.json`: 3 report rows,
    `sizes_match=true`.
  - `provisional/ph34-multi-scope-provisional-readback.json`:
    `provisional=true`.
  - `edges/ph34-multi-scope-edges-readback.json`: empty intersect size `0`,
    temporal error `CALYX_SCOPE_TEMPORAL_NOT_READY`, cache unchanged by error.
  - `anchors/ph34-multi-scope-anchors-readback.json`: collection anchor count
    `1`, tenant anchor count `0`.
- **Identity follow-up (#328):**
  `/home/croyse/calyx/data/fsv-issue328-scope-cache-identity-20260608/scope-cache-identity/ph34-scope-cache-identity-readback.json`
  proves miss/hit behavior from the real `build_kernel` path.

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) <= 500 lines
- [x] FSV evidence attached to the PH34/#328 GitHub issues
- [x] no anti-pattern: no flatten / no ungrounded trusted state / no harness-only FSV
