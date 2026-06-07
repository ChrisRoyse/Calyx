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

## Build (checklist of concrete, code-level steps)

- [ ] `pub fn build_kernel(vault: &mut Vault, scope: Scope, anchor_kind: Option<AnchorKind>, params: KernelParams, cache: &mut ScopeCache) -> Result<Kernel, CalyxError>`:
  1. Compute `key = ScopeCacheKey { scope_hash: scope_hash(&scope), panel_version: vault.panel_version() }`.
  2. Cache hit → return cloned `Kernel`.
  3. Cache miss → `materialize_scope(&scope, vault)` → `subgraph`;
     `build_kernel_pipeline(&subgraph, &anchors_for_scope(&scope, vault, anchor_kind), &params)` → `kernel`;
     `cache.insert(key, kernel.clone())`;
     return `kernel`.
- [ ] `pub fn anchors_for_scope(scope: &Scope, vault: &Vault, anchor_kind: Option<AnchorKind>) -> Vec<CxId>` — selects the anchors relevant to the scope (same collection/tenant/domain filter applied to anchors).
- [ ] `pub struct ScopeKernelReport { scope_name: String, scope_hash: [u8;32], kernel_size: usize, kernel_graph_size: usize, kernel_only_recall: f32, grounded_fraction: f32, approx_factor: f64 }`.
- [ ] `pub fn report_all_scopes(kernels: &[(Scope, Kernel)]) -> Vec<ScopeKernelReport>` — collects one `ScopeKernelReport` per scope; values come from `Kernel.recall` and `Kernel.groundedness`; no re-computation.
- [ ] Ungrounded scope kernel (grounded_fraction < some epsilon, e.g. 0.01) →
  emit `CALYX_KERNEL_UNGROUNDED` in `Kernel.estimator_provenance` + tag `"provisional"`.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: 2-scope test — `AllAssociations` and `Collection(id1)` on a 10-node store;
  both produce non-empty kernels; the `Collection` kernel is a subset of the
  `AllAssociations` kernel (members ⊆).
- [ ] unit: cache hit — call `build_kernel` twice with the same scope and same
  `panel_version`; `cache.stats().hits == 1` on the second call.
- [ ] unit: `report_all_scopes` on 3 kernels → 3 `ScopeKernelReport` rows;
  `kernel_size` values match `kernel.members.len()` for each.
- [ ] unit: scope with 0 anchors → `Kernel.estimator_provenance` contains `"provisional"`;
  `CALYX_KERNEL_UNGROUNDED` in provenance string.
- [ ] edge: `panel_version` bumped between two calls → cache miss on second call
  (different key); new kernel computed.
- [ ] edge: `Intersect(A, B)` scope that resolves to empty graph → `Kernel.members = []`;
  `ScopeKernelReport.kernel_size = 0`; no panic.
- [ ] fail-closed: `materialize_scope` returns `CALYX_SCOPE_TEMPORAL_NOT_READY` →
  propagated from `build_kernel`; cache not populated.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `cargo test -p calyx-lodestar multi_scope -- --nocapture` stdout.
- **Readback:** `cargo test -p calyx-lodestar multi_scope 2>&1 | tee /tmp/ph34_t03_fsv.txt && cat /tmp/ph34_t03_fsv.txt`.
- **Prove:** 2-scope test prints `AllAssociations` kernel size ≥ `Collection` kernel
  size; cache hit test prints `hits = 1`; provisional test prints `"provisional"` in
  provenance; all tests pass; output attached to PH34 GitHub issue.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH34 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
