# PH34 · T04 — Hierarchical kernel-of-regions for huge scopes

| Field | Value |
|---|---|
| **Phase** | PH34 — Multi-scope kernel |
| **Stage** | S6 — Lodestar Kernel |
| **Crate** | `calyx-lodestar` |
| **Files** | `crates/calyx-lodestar/src/hierarchical.rs` (≤500) |
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

## Build (checklist of concrete, code-level steps)

- [ ] `pub struct HierarchicalKernel { region_kernel: Kernel, region_drilldowns: Vec<(RegionId, Kernel)> }`.
- [ ] `pub fn build_hierarchical_kernel(vault: &mut Vault, scope: Scope, params: &HierarchicalKernelParams, cache: &mut ScopeCache) -> Result<HierarchicalKernel, CalyxError>`:
  1. Get named regions for the scope: `vault.list_regions(scope)`.
  2. Build region-level graph: each region is a node; edges = inter-region association
     edge density (sum of edge weights between regions normalized by region size).
  3. `build_kernel_pipeline(&region_graph, ...)` → `region_kernel`.
  4. For each region in `region_kernel.members`, drill down:
     `build_kernel(&region_scope, ...)` where `region_scope = Subgraph { query: region.centroid_cx, radius: params.drill_radius }`.
  5. Return `HierarchicalKernel`.
- [ ] `pub struct HierarchicalKernelParams { max_regions: usize, drill_radius: usize, min_region_size: usize }`.
- [ ] If `vault.list_regions` returns 0 regions → fall back to `build_kernel(scope, ...)` directly (no error).
- [ ] `HierarchicalKernel` exposes `all_members() -> Vec<CxId>` = union of all drilldown members.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: store with 3 regions (10 nodes each, known inter-region edges);
  `build_hierarchical_kernel` → `region_kernel.members.len() ≤ 3`;
  at least 1 region drilldown is populated.
- [ ] unit: `all_members()` count ≤ sum of drilldown `kernel.members.len()` (some
  overlap possible via union; no duplicates in the returned vec).
- [ ] unit: 0 regions → falls back to `build_kernel(AllAssociations)` with no error.
- [ ] unit: `build_hierarchical_kernel` twice with same inputs and `panel_version` →
  second call hits cache for all drilldowns; `cache.stats().hits > 0`.
- [ ] edge: a region with 1 node → drilldown returns 0 or 1 members (not a panic).
- [ ] edge: `max_regions = 1` → only 1 region kernel computed; 1 drilldown.
- [ ] fail-closed: `drill_radius = 0` → drilldown subgraph = single node = 0-member kernel;
  `HierarchicalKernel.region_drilldowns[0].1.members = []`; no panic.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `cargo test -p calyx-lodestar hierarchical -- --nocapture` stdout.
- **Readback:** `cargo test -p calyx-lodestar hierarchical 2>&1 | tee /tmp/ph34_t04_fsv.txt && cat /tmp/ph34_t04_fsv.txt`.
- **Prove:** 3-region test prints `region_kernel.members.len() ≤ 3` and at least 1
  drilldown entry; cache-hit test prints `hits > 0`; zero-region fallback runs without
  error; output attached to PH34 GitHub issue.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH34 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
