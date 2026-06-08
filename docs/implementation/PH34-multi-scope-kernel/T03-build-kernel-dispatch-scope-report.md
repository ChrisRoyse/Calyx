# PH34 · T03 — `build_kernel(scope, ...)` dispatch + per-scope recall + grounded-fraction

| Field | Value |
|---|---|
| **Phase** | PH34 — Multi-scope kernel |
| **Stage** | S6 — Lodestar Kernel |
| **Crate** | `calyx-lodestar` |
| **Files** | `crates/calyx-lodestar/src/multi_scope.rs` (≤500), `crates/calyx-lodestar/src/scope_report.rs` (≤500) |
| **Depends on** | T01 (`materialize_scope`), T02 (`ScopeCache`) |
| **Axioms** | A21, A10 |
| **PRD** | `dbprdplans/08 §4b`, `08 §8` |

## Goal

Implement the top-level `build_kernel(vault, scope, anchor_kind?, params?) -> Kernel`
function described in `08 §8`. This dispatches through the scope cache (T02), calls
`materialize_scope` on a miss, and runs `build_kernel_pipeline` on the resulting
subgraph. Each scope's `Kernel` carries its own measured `recall` and
`groundedness` (never assumed from global stats). The `ScopeKernelReport` struct
aggregates results across scopes for observability.

## Status

Implemented in issue #235. aiwonder FSV readbacks live under
`/home/croyse/calyx/data/fsv-issue235-multi-scope-20260608`; the serial FSV log
is `ph34_t03_fsv.log`.

## Build (checklist of concrete, code-level steps)

- [x] `pub fn build_kernel(store: &dyn AssocStore, scope: Scope, anchor_kind: Option<AnchorKind>, params: KernelParams, cache: &mut ScopeCache) -> Result<Kernel>`:
  1. Compute `key = ScopeCacheKey { scope_hash: scope_hash(&scope), panel_version: params.panel_version }`.
  2. Cache hit -> return cloned `Kernel`.
  3. Cache miss -> `materialize_scope(&scope, store)` -> `subgraph`;
     `build_kernel_pipeline(&subgraph, &anchors_for_scope(&scope, store, anchor_kind), &params)` -> `kernel`;
     `cache.insert(key, kernel.clone())`;
     return `kernel`.
- [x] `pub fn anchors_for_scope(scope: &Scope, store: &dyn AssocStore, anchor_kind: Option<AnchorKind>) -> Vec<CxId>` — selects the anchors relevant to the scope (same collection/tenant/domain filter applied to anchors).
- [x] `pub struct ScopeKernelReport { scope_name: String, scope_hash: [u8;32], kernel_size: usize, kernel_graph_size: usize, kernel_only_recall: f32, grounded_fraction: f32, approx_factor: f64 }`.
- [x] `pub fn report_all_scopes(kernels: &[(Scope, Kernel)]) -> Vec<ScopeKernelReport>` — collects one `ScopeKernelReport` per scope; values come from `Kernel.recall` and `Kernel.groundedness`; no re-computation.
- [x] Ungrounded scope kernel (grounded_fraction < some epsilon, e.g. 0.01) ->
  emit `CALYX_KERNEL_UNGROUNDED` in `Kernel.estimator_provenance` + tag `"provisional"`.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: 2-scope test — `AllAssociations` and `Collection(id1)` on a known cyclic store;
  both produce non-empty kernels; the `Collection` kernel is a subset of the
  `AllAssociations` kernel (members ⊆).
- [x] unit: cache hit — call `build_kernel` twice with the same scope and same
  `panel_version`; `cache.stats().hits == 1` on the second call.
- [x] unit: `report_all_scopes` on 3 kernels → 3 `ScopeKernelReport` rows;
  `kernel_size` values match `kernel.members.len()` for each.
- [x] unit: scope with 0 anchors → `Kernel.estimator_provenance` contains `"provisional"`;
  `CALYX_KERNEL_UNGROUNDED` in provenance string.
- [x] edge: `panel_version` bumped between two calls → cache miss on second call
  (different key); new kernel computed.
- [x] edge: `Intersect(A, B)` scope that resolves to empty graph → `Kernel.members = []`;
  `ScopeKernelReport.kernel_size = 0`; no panic.
- [x] fail-closed: `materialize_scope` returns `CALYX_SCOPE_TEMPORAL_NOT_READY` →
  propagated from `build_kernel`; cache not populated.

## FSV (read the bytes on aiwonder — the truth gate)

- **Trigger:** `CALYX_FSV_ROOT=/home/croyse/calyx/data/fsv-issue235-multi-scope-20260608 cargo test -p calyx-lodestar --test ph34_multi_scope_tests -- --nocapture --test-threads=1`
- **SoT readbacks:**
  - `subset/ph34-multi-scope-subset-readback.json`: `all_size_gte_collection=true`,
    `collection_subset=true`, all members `[01.., 04..]`, collection member `[01..]`.
  - `cache/ph34-multi-scope-cache-readback.json`: `hits=1`, `misses=1`,
    `current_size=1`.
  - `reports/ph34-multi-scope-reports-readback.json`: 3 report rows,
    `sizes_match=true`, per-scope grounded fractions read from `Kernel`.
  - `provisional/ph34-multi-scope-provisional-readback.json`: `provisional=true`,
    provenance contains `CALYX_KERNEL_UNGROUNDED`, panel bump stats `misses=2`.
  - `edges/ph34-multi-scope-edges-readback.json`: empty intersect report size `0`,
    temporal error `CALYX_SCOPE_TEMPORAL_NOT_READY`, cache size unchanged by error.
  - `anchors/ph34-multi-scope-anchors-readback.json`: collection anchor count `1`,
    tenant anchor count `0`.
- **Hashes:** subset
  `9ca8215bad4e61df0cf1b395155b4d826d3b13725828939f241e1ab1d7d3eeb5`;
  cache `cc090db35d729295e9234efd6ed3a2908b5a05abab79540e7f6be6f1016e266b`;
  reports `d02c8a83ac660bb635cd4234963bdffc272f07ae044636af270cc320e4c17612`;
  provisional `e673f5bc09954233a553d61126286ff4a7930e067ad465931e024c0b3e9d27f2`;
  edges `7c7aa3fa1537ef30929532b77e0f789bccffb86fe90c8a074ddb59a8cd20f37f`;
  anchors `da16993800ad850977454fbbe6d1d39ce6db71add3f4dd1acb91b1544111dddb`;
  log `74aa331c90c65d0e22fd34a0ff97a955e2a172e46782577c2de36df839b83544`.

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] FSV evidence (readback output / screenshot) attached to the PH34 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
