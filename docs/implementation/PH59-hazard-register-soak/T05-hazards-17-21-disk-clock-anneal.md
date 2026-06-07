# PH59 · T05 — Hazards 17–21: disk full, ARC thrash, clock skew, Anneal thrash, panel explosion

| Field | Value |
|---|---|
| **Phase** | PH59 — 25-hazard register FSV + soak |
| **Stage** | S13 — Resource, GC & Reliability Hardening |
| **Crate** | `calyx-hazard-soak` |
| **Files** | `crates/calyx-hazard-soak/src/hazards/operational.rs` (≤500, continued) |
| **Depends on** | PH56 T06 (disk-pressure guard), PH43 (Anneal tripwires), PH58 T05 (panel GC) |
| **Axioms** | A26, A16 |
| **PRD** | `dbprdplans/24 §7` hazards 17–21 |

## Goal

Drive hazards 17 (disk full on `hotpool`), 18 (ZFS ARC pressure / mmap thrash), 19 (clock
skew — server-stamped monotonic seq), 20 (Anneal thrash / oscillation), and 21 (panel-version
/ cross-term explosion), read the SoT bytes, prove each mitigation. Hazard 17 is critical for
the single-NVMe `hotpool` (no redundancy). Hazard 19 must prove seq-based ordering is
invariant to wall-clock skew.

## Build (checklist of concrete, code-level steps)

**Hazard 17 — Disk full on `hotpool`:**
- [ ] `fn probe_h17_disk_full(vault: &mut Vault, guard: &DiskPressureGuard) -> HazardResult`:
  - Fill `hotpool` to 87% using a temporary test file (`dd if=/dev/zero of=/hotpool/fill_test bs=1M count=N`)
  - Attempt a write to the vault; verify `CALYX_DISK_PRESSURE` returned (not a panic or filesystem error)
  - Verify no SST corruption (`fsck`-equivalent: all existing CxIds still readable)
  - Delete the fill file; verify writes succeed again
  - Record `disk_pressure_events_total` increment and `disk_free` before/after

**Hazard 18 — ZFS ARC pressure / mmap thrash:**
- [ ] `fn probe_h18_arc_thrash(vault: &Vault) -> HazardResult`:
  - Shrink ARC max to 512 MiB: `echo 536870912 > /sys/module/zfs/parameters/zfs_arc_max`
  - Repeatedly read 4 GB of mmap'd column data in random order (working-set larger than ARC)
  - Verify: no thrash collapse — read throughput does not drop to 0; degrades gracefully ≥ 10% of unconstrained throughput
  - Restore ARC max to normal; verify recovery
  - Record `arc_pressure_events` (ZFS kstat: `kstat -n zarcstats | grep hits`)

**Hazard 19 — Clock skew:**
- [ ] `fn probe_h19_clock_skew(vault: &mut Vault, clock: &MockClock) -> HazardResult`:
  - Write 100 entries with seq 1–100 using the injected `Clock`
  - Advance wall clock backward by 30 s (simulate NTP correction)
  - Write 100 more entries; verify their seq numbers are 101–200 (monotonic seq, not wall-clock-based)
  - Verify range scan returns entries in seq order, not wall-clock order
  - Record `clock_skew_ordering_violations == 0`

**Hazard 20 — Anneal thrash / oscillation:**
- [ ] `fn probe_h20_anneal_thrash(anneal: &Anneal) -> HazardResult`:
  - Run 1e6 queries with an adversarial workload designed to flip the autotune config repeatedly
  - Verify: hysteresis and tripwires prevent more than K config flips (K = configured threshold, default 3/hour)
  - Verify: shadow-first testing (new config runs in shadow before flip); no rollback storm
  - Record `anneal_config_flips_total ≤ K`; `anneal_rollback_total` stable (not monotonically rising)

**Hazard 21 — Panel-version / cross-term explosion:**
- [ ] `fn probe_h21_panel_explosion(vault: &mut Vault) -> HazardResult`:
  - Add and retire 20 lenses in rapid succession; each add/retire creates a new panel version
  - Verify `PanelVersionGc` runs and prunes unreferenced versions; `panel_versions_live ≤ hot_versions_to_keep + referenced`
  - Verify `n_eff` budget caps materialized cross-terms (< C(N,2)); check `materialized_xterms ≤ n_eff_budget`
  - Record `panel_versions_pruned_total > 0`; `materialized_xterms`

- [ ] Aggregate into `target/ph59_hazards_17_21.json`

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] H17: `disk_pressure_events_total >= 1` after fill; all pre-fill CxIds readable (no corruption); writes succeed after fill file removed
- [ ] H18: read throughput at 512 MiB ARC ≥ 10% of unconstrained (no total collapse); ARC restored; throughput recovers
- [ ] H19: `clock_skew_ordering_violations == 0`; seq 101 > seq 100 regardless of wall-clock direction
- [ ] H20: `anneal_config_flips_total ≤ 3` over 1e6 queries; no oscillation (flip/unflip/flip pattern absent)
- [ ] H21: `panel_versions_live ≤ hot_versions_to_keep + 2` (2 currently referenced); `panel_versions_pruned_total >= 15` (of 20 added)
- [ ] edge: H17 — fill to exactly 85% (high-water boundary) → `CALYX_DISK_PRESSURE` fires on next write; 84% → no fire
- [ ] edge: H19 — wall clock moves forward by 1 year → seq still monotonically increments (no integer overflow; check u64 seq space)
- [ ] fail-closed: H20 — if Anneal thrash is unchecked (hysteresis disabled for test), `anneal_rollback_total` grows linearly; with hysteresis, it saturates

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `target/ph59_hazards_17_21.json`; `calyx readback --metric disk_pressure_events_total`; `calyx readback --metric anneal_config_flips_total`; `calyx readback --metric panel_versions_pruned_total`
- **Readback:**
  ```
  calyx readback --metric disk_pressure_events_total
  calyx readback --metric anneal_config_flips_total
  calyx readback --metric panel_versions_pruned_total
  df -h /hotpool
  cat target/ph59_hazards_17_21.json | python3 -c "import json,sys; d=json.load(sys.stdin); print('passed:', all(h['passed'] for h in d))"
  ```
- **Prove:** all five hazards report `passed: true`; `disk_pressure_events_total >= 1`; `clock_skew_ordering_violations == 0`; `anneal_config_flips_total ≤ 3`; `panel_versions_pruned_total >= 15`. Attach JSON + readbacks to PH59 GitHub issue.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH59 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
