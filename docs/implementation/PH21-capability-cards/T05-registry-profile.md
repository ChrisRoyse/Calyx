# PH21 T05 - Registry profile + Assay-backed capability metrics

| Field | Value |
|---|---|
| **Phase** | PH21 - Capability cards / profile |
| **Stage** | S3 - Registry / Lenses |
| **Crate** | `calyx-registry` |
| **Files** | `crates/calyx-registry/src/profile.rs`, `src/profile/assay.rs`, `src/panel_ops.rs` |
| **Depends on** | T02, T03, T04, Stage 5 Assay |
| **Axioms** | A6, A17 |
| **PRD** | `dbprdplans/05` section 5 |
| **Status** | DONE / FSV-signed-off in #334 |

## Goal

Produce a capability card that keeps fast Registry probe metrics explicit as
proxies while allowing grounded Stage 5 Assay rows to populate `signal`,
`differentiation`, and panel `bits_about` when a scoped `AssayStore` is
available.

## Build

- [x] `profile_lens` profiles a registered lens with deterministic probe
  vectors, coverage, spread, separation, cost, and low-spread detection.
- [x] No-Assay callers get `signal:null`, `differentiation:null`,
  `signal_source:"assay_pending"`, and `differentiation_source:"assay_pending"`.
- [x] `profile_slot_with_assay` overlays scoped `AssayStore` lens rows as
  `signal` and pair-gain rows as `differentiation`, both sourced as
  `assay_store`.
- [x] `list_panel` exposes already-stored `Slot.bits_about` values.
- [x] `list_panel_with_assay` overlays scoped Assay lens rows as panel
  `bits_about`.

## Tests

- [x] Algorithmic lens profile returns finite proxy/spread/separation/cost
  metrics and pending Assay-owned fields.
- [x] Assay rows attach `signal=0.42` and pair-gain differentiation `0.07`
  in unit coverage.
- [x] Panel listing reads stored slot bits and Assay overlay bits.
- [x] Collapsed lens is flagged low-spread.
- [x] Empty probes fail closed with `CALYX_ASSAY_INSUFFICIENT_SAMPLES`.
- [x] Mixed-modality probes report partial coverage without inventing Assay
  fields.

## FSV

- **Evidence root:** `/home/croyse/calyx/data/fsv-issue334-ph21-assay-registry-20260608`
- **Trigger:** `CALYX_FSV_ROOT=<root> cargo test -p calyx-registry ph21_profile_card_aiwonder_fsv -- --ignored --nocapture`
- **Before readback:** `02-before-pending-card.out` shows `signal:null`,
  `differentiation:null`, and both sources as `assay_pending`.
- **After readback:** `03-assay-backed-card.out` shows `signal:0.42`,
  `signal_source:"assay_store"`, `differentiation:0.08`, and
  `differentiation_source:"assay_store"`.
- **Panel readback:** `04-assay-backed-panel-listing.out` shows
  `bits_about:0.42`.
- **SoT bytes:** `05-assay-cf-json-readback.out` and `07-assay-sst-prefix.hex`
  show the persisted Assay CF rows for the lens and pair subjects.
- **Gates:** `final-gates/10-fmt-check.out` through `14-linecount.out` record
  aiwonder fmt, check, test, clippy, and line-count gates.

## Done

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder.
- [x] File(s) <= 500 lines.
- [x] FSV evidence attached to issue #334.
- [x] No proxy metric is promoted to grounded Assay quality without a stored
  scoped Assay row.
