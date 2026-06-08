# PH34 · T04 — Hierarchical kernel-of-regions for huge scopes

| Field | Value |
|---|---|
| **Phase** | PH34 — Multi-scope kernel |
| **Stage** | S6 — Lodestar Kernel |
| **Crate** | `calyx-lodestar` |
| **Files** | `crates/calyx-lodestar/src/hierarchical.rs` (<=500), `crates/calyx-lodestar/tests/ph34_hierarchical_tests.rs` (<=500) |
| **Depends on** | T03 (`build_kernel`, `ScopeCache`), PH09 (named region / cluster concepts) |
| **Axioms** | A21, A10 |
| **PRD** | `dbprdplans/08 §3` ("For huge vaults, kernel-of-regions → region → constellation is a 3-hop funnel"), `08 §4b` ("Nested & incremental. Hierarchical (kernel-of-kernels)") |

## Goal

Implement `build_hierarchical_kernel`: for huge scopes (e.g. `AllAssociations` on
a billion-node corpus) where running `build_kernel_pipeline` directly is intractable,
first compute a kernel **of named regions** (clusters from PH09 / named ConceptSpaces),
then drill down into the kernel of a single region. The result is a `HierarchicalKernel`
with a two-level structure: region-level members and constellation-level members within
each region.

## Status

Implemented and FSV-signed-off on aiwonder on 2026-06-08 for #236. The
implementation adds `RegionStore`, deterministic `RegionId` region-node IDs,
bounded region selection, inter-region edge density, scoped anchor projection,
fallback-to-direct kernel behavior, and cache-backed drilldowns through the
existing PH34 `build_kernel` dispatch.

## Build (checklist of concrete, code-level steps)

- [x] `pub struct HierarchicalKernel { region_kernel: Kernel, region_drilldowns: Vec<(RegionId, Kernel)> }`.
- [x] `pub fn build_hierarchical_kernel(store: &dyn RegionStore, scope: Scope, params: &HierarchicalKernelParams, cache: &mut ScopeCache) -> Result<HierarchicalKernel>`:
  1. Get named regions for the scope: `store.regions_for_scope(&scope)`.
  2. Build region-level graph: each region is a node; edges = inter-region association
     edge density (sum of edge weights between regions normalized by region size).
  3. `build_kernel_pipeline(&region_graph, ...)` -> `region_kernel`.
  4. For each selected region in `region_kernel.members`, drill down:
     `build_kernel(&region_scope, ...)` where `region_scope = Subgraph { query: region.centroid_cx, radius: params.drill_radius }`.
  5. Return `HierarchicalKernel`.
- [x] `pub struct HierarchicalKernelParams { max_regions: usize, drill_radius: usize, min_region_size: usize, anchor_kind: Option<AnchorKind>, kernel_params: KernelParams }`.
- [x] If `RegionStore::regions_for_scope` returns 0 regions -> fall back to `build_kernel(scope, ...)` directly (no error).
- [x] `HierarchicalKernel` exposes `all_members() -> Vec<CxId>` = union of all drilldown members.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: store with 3 regions (3 nodes each, known inter-region edges);
  `build_hierarchical_kernel` -> `region_kernel.members.len() <= 3`;
  at least 1 region drilldown is populated.
- [x] unit: `all_members()` count <= sum of drilldown `kernel.members.len()` (some
  overlap possible via union; no duplicates in the returned vec).
- [x] unit: 0 regions -> falls back to `build_kernel(AllAssociations)` with no error.
- [x] unit: `build_hierarchical_kernel` twice with same inputs and `panel_version` ->
  second call hits cache for all drilldowns; `cache.stats().hits > 0`.
- [x] edge: a region with 1 node -> drilldown returns 0 or 1 members (not a panic).
- [x] edge: `max_regions = 1` -> only 1 region kernel computed; 1 drilldown.
- [x] fail-closed: `drill_radius = 0` -> drilldown subgraph = single node = 0-member kernel;
  `HierarchicalKernel.region_drilldowns[0].1.members = []`; no panic.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** JSON readback files written by the PH34 T04 tests under
  `/home/croyse/calyx/data/fsv-issue236-hierarchical-20260608` on aiwonder.
- **Trigger:** `CALYX_FSV_ROOT=/home/croyse/calyx/data/fsv-issue236-hierarchical-20260608 cargo test -p calyx-lodestar --test ph34_hierarchical_tests -- --nocapture --test-threads=1`.
- **Readbacks and sha256:**
  - `regions/ph34-hierarchical-regions-readback.json`:
    `0ac12c345f1eb4e035e34bfdf4560d0ac64dad08c8543f0e9c01adac345aa926`
  - `members/ph34-hierarchical-members-readback.json`:
    `ea1fe330090a07fd646e870f16a81f5c26da106edf04643def9d24b98f97f8a8`
  - `fallback/ph34-hierarchical-fallback-readback.json`:
    `dac605e83853cde5ed057a89c6f52a223314f85e6b148866bd304ed8459b18ac`
  - `cache/ph34-hierarchical-cache-readback.json`:
    `322fb7b50262b86f48c466d165c921ece82e488527e6f9c638f5f3abe7ecc2c9`
  - `edges/ph34-hierarchical-edges-readback.json`:
    `9fe451d9a86f4b936e83ebd49430c9f84356f1326826b5258f23c91476df2757`
  - FSV log:
    `0c559b4aef708fcd2111ca3d19bacfefaca5e3e4103e6428812b9321ceb05707`
- **Prove:** 3-region build has `region_kernel_size <= 3` and a populated
  drilldown, `all_members` dedupes drilldowns, zero regions falls back to direct
  kernel with no drilldowns, repeated build records cache hits, and
  single-region / `max_regions = 1` / `drill_radius = 0` edge cases do not panic.

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) <= 500 lines (line-count gate green)
- [x] FSV evidence (readback output / screenshot) attached to the PH34 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
