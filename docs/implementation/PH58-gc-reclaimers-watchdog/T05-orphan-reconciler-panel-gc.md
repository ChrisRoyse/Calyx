# PH58 · T05 — Orphan reconciler + panel/codebook version GC

| Field | Value |
|---|---|
| **Phase** | PH58 — GC reclaimers + long-reader watchdog + janitor |
| **Stage** | S13 — Resource, GC & Reliability Hardening |
| **Crate** | `calyx-aster` |
| **Files** | `crates/calyx-aster/src/gc/orphan_reconciler.rs` (≤500), `crates/calyx-aster/src/gc/panel_version_gc.rs` (≤500) |
| **Depends on** | T03 (rate-limit pattern), T04 (GC scheduler) |
| **Axioms** | A26, A16 |
| **PRD** | `dbprdplans/24 §3`, `24 §7` hazard 21 |

## Goal

Implement the orphan reconciler: periodic scan for index entries with no corresponding base
constellation (and vice versa — base CxIds with no slot column entries) → repair or purge.
Inherited from Leapable's `reconcile_files` discipline. Also implement panel/codebook version
GC: immutable panel versions and old codebook versions that are no longer referenced by any
live constellation are pruned (cold-tier first); retired-lens slot columns are retained for
historical interpretability then pruned by retention policy. Both reclaimers are rate-limited
and reversible within the recovery window. Defends hazard 21 (panel-version/cross-term
explosion).

## Build (checklist of concrete, code-level steps)

**Orphan Reconciler (`orphan_reconciler.rs`):**
- [ ] Define `struct OrphanReconciler { scan_interval: Duration, max_repairs_per_run: usize }`
- [ ] Implement `OrphanReconciler::scan(&self, vault: &Vault) -> OrphanReport` — scans `base` CF and all `slot_*` CFs; finds CxIds present in an index but absent in base (orphan index entries) and CxIds present in base but absent in all expected slot CFs (orphan base entries); returns `OrphanReport { orphan_index: Vec<CxId>, orphan_base: Vec<CxId>, inconsistencies: usize }`
- [ ] Implement `OrphanReconciler::repair(&self, vault: &mut Vault, report: &OrphanReport) -> GcResult` — for orphan index entries: delete the index entry + emit Ledger entry; for orphan base entries: flag as `degraded` + trigger slot column rebuild (delegate to Anneal); rate-limited to `max_repairs_per_run`
- [ ] Emit Prometheus: `calyx_orphan_index_entries_total`, `calyx_orphan_base_entries_total`, `calyx_orphan_repairs_total`

**Panel Version GC (`panel_version_gc.rs`):**
- [ ] Define `struct PanelVersionGc { retention_policy: RetentionPolicy }` where `RetentionPolicy` specifies `hot_versions_to_keep: usize` and `cold_tier_first: bool`
- [ ] Implement `PanelVersionGc::find_unreferenced(&self, vault: &Vault) -> Vec<PanelVersionId>` — scans all live constellation slot entries for panel version references; collects panel versions with zero references; filters by `hot_versions_to_keep` (keep the last N versions even if unreferenced, for interpretability)
- [ ] Implement `PanelVersionGc::prune(&self, vault: &mut Vault, ids: &[PanelVersionId]) -> GcResult` — moves unreferenced versions from hot to cold tier first (`cold_tier_first=true`); then drops from cold after a second retention period; never touches Ledger entries
- [ ] Implement `RetiredLensGc::prune_retired(&self, vault: &mut Vault, lens_id: LensId) -> GcResult` — for a retired lens, moves its slot columns to cold tier; after retention period, purges; retains historical interpretability rows per policy
- [ ] Emit Prometheus: `calyx_panel_versions_pruned_total`, `calyx_panel_versions_live`, `calyx_retired_lens_bytes_freed_total`

## Tests (synthetic, deterministic — known input → known bytes/number)

**Orphan Reconciler:**
- [ ] unit: create 5 base CxIds; create slot entries for only 3; `scan` → `orphan_base == [cx4, cx5]`; `repair` flags them `degraded` and triggers rebuild
- [ ] unit: create 3 orphan index entries (no base) → `orphan_index == [cx6, cx7, cx8]`; `repair` removes index entries and writes Ledger entries (3 new Ledger rows)
- [ ] unit: rate limit — 10 orphans, `max_repairs_per_run=3` → exactly 3 repaired; 7 remain for next run

**Panel Version GC:**
- [ ] unit: create 5 panel versions; only version 3 referenced by a live constellation; `find_unreferenced` returns [1,2,4,5] but `hot_versions_to_keep=2` → returns [1,2] only (keep 4,5 as most recent N)
- [ ] unit: `prune([1,2])` → moves to cold tier first; second call (simulated second retention period) → purges; `panel_versions_pruned_total == 2`
- [ ] proptest: `forall panel_versions, live_refs` — unreferenced never includes any version with a live reference
- [ ] edge: all panel versions referenced → `find_unreferenced` returns empty; no pruning
- [ ] fail-closed: prune encounters a Ledger entry for the panel version → skip that version (Ledger is append-only; never delete Ledger-referenced data); emit warning log

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `calyx_orphan_repairs_total`, `calyx_panel_versions_pruned_total`, and `du -sh /hotpool/vault_*/panels` on aiwonder
- **Readback:**
  ```
  calyx readback --metric orphan_repairs_total
  calyx readback --metric panel_versions_pruned_total
  du -sh /hotpool/vault_*/panels
  ```
- **Prove:** ingest data with deliberate orphans (write base CxIds without slot entries); run `OrphanReconciler`; verify `orphan_repairs_total > 0`; add 10 panel versions, reference only 2; run `PanelVersionGc`; verify `panel_versions_pruned_total >= 6` (depending on `hot_versions_to_keep`); `du` output decreases. Attach readback output to PH58 GitHub issue.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH58 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
