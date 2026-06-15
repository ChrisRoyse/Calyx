# PH11 · T03 — Tiering policy: hot/cold CF placement + staging-in-dest

| Field | Value |
|---|---|
| **Phase** | PH11 — Compaction + hot/cold tiering |
| **Stage** | S1 — Aster storage core |
| **Crate** | `calyx-aster` |
| **Files** | `crates/calyx-aster/src/compaction/mod.rs` (≤500), `crates/calyx-aster/src/compaction/tests.rs` (≤500) |
| **Depends on** | T02 (CompactionCatalog), PH07 T03 (CF naming) |
| **Axioms** | A26 |
| **PRD** | `dbprdplans/04 §3`, `dbprdplans/04 §6` |

## Goal

Prove that `TieringPolicy::place_cf` correctly classifies CFs as hot or cold:
active-slot quantized columns and `base`/`ledger`/`anchors` CFs are hot
(`/zfs/hot/calyx/cf/<name>`); `slot_*.raw` sidecars, inactive-slot quantized
columns, and old panel versions are cold (`/zfs/archive/calyx/cf/<name>`). Also
prove that `write_tiered_sst` stages temp files inside the destination dataset
(same directory) to avoid `EXDEV` on ZFS rename.

## Build (checklist of concrete, code-level steps)

- [x] Add test: `TieringPolicy::aiwonder(active_slots=[0,1], panel_version=3)
  .place_cf(CF::slot(SlotId(0)), 3).tier == StorageTier::Hot`.
- [x] Add test: `place_cf(CF::slot(SlotId(0)), 2).tier == StorageTier::Cold`
  (old panel version).
- [x] Add test: `place_cf(CF::slot_raw(SlotId(0)), 3).tier == StorageTier::Cold`
  (`*.raw` is always cold).
- [x] Add test: `place_cf(CF::slot(SlotId(2)), 3).tier == StorageTier::Cold`
  (slot 2 not in active_slots).
- [x] Add test: `place_cf(CF::Base, 3).tier == StorageTier::Hot`.
- [x] Add test: `place_cf(CF::Ledger, 3).tier == StorageTier::Hot`.
- [x] Add proptest: for any `(cf, panel_version, active_slots)` combination, the
  returned `TierPlacement.absolute_dir()` starts with the correct root path.
- [x] Verify `write_tiered_sst` calls `write_sst` which uses
  `path.with_extension("sst.tmp")` in the same directory as the output — confirm
  the temp file is created in `placement.absolute_dir()` (not in `/tmp`).
- [x] Add fallback for missing ZFS paths: if `/zfs/hot/calyx` or
  `/zfs/archive/calyx` do not exist, fall back to
  `$CALYX_HOME/hot/cf/` and `$CALYX_HOME/archive/cf/` respectively; log a
  warning.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: 6 placement cases above.
- [x] proptest: `absolute_dir` starts with correct root.
- [x] unit: `write_tiered_sst` creates a file at `placement.absolute_dir() /
  <filename>` (in a tempdir); no `EXDEV` because temp file is in the same dir.
- [x] edge (≥3): (1) no active slots → all slot CFs cold; (2) all slot CFs
  active → all hot; (3) `panel_version = current_panel_version - 1` → cold.
- [x] fail-closed: `write_tiered_sst` on a read-only directory → `CALYX_DISK_PRESSURE`.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** Presence of SST files in hot vs cold tier directories.
- **Readback:**
  ```
  calyx tier --vault /home/croyse/calyx/test-vault --cf slot_00 --output cold
  ls /home/croyse/calyx/archive/cf/slot_00/
  ls /home/croyse/calyx/hot/cf/base/
  ```
- **Prove:** Cold-tier SST file exists at `archive/cf/slot_00/`; hot-tier SST
  file exists at `hot/cf/base/`. No temp files (`*.sst.tmp`) remain after the
  write. Screenshot posted to PH11 GitHub issue.

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] FSV evidence (readback output / screenshot) attached to the PH11 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
