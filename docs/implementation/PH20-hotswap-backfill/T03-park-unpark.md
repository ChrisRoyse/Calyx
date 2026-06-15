# PH20 · T03 — park_lens / unpark_lens

| Field | Value |
|---|---|
| **Phase** | PH20 — Hot-swap add/retire/park + lazy backfill |
| **Stage** | S3 — Registry / Lenses |
| **Crate** | `calyx-registry` |
| **Files** | `crates/calyx-registry/src/swap.rs` (≤500) |
| **Depends on** | T01 (this phase) |
| **Axioms** | A5 |
| **PRD** | `dbprdplans/05 §8` (API summary: `park_lens / unpark_lens`) |

## Goal

Implement `park_lens(slot_id)` and `unpark_lens(slot_id)`. Parked means: keep
the slot and its data, do not measure it on new constellations, do not include
it in search — low-signal / suspended. Unparking restores it to `Active` and
re-enqueues backfill for any constellations added while it was parked. Both
state-changing operations bump `panel_version`; repeated no-op calls do not.

Post-sweep #327: `SwapController::park_lens` and `unpark_lens` are idempotent
when the slot is already in the requested state. `park_lens` cancels pending
in-memory backfill for the slot; `unpark_lens` restores the state to Active
without fabricating a synthetic backfill request. Full rescan/watermark backfill
after unpark remains a later scheduler-policy extension. The current core error
catalog uses `CALYX_LENS_FROZEN_VIOLATION` for unknown or retired lifecycle
requests; a registry-specific not-found code would be a later catalog expansion.

## Build (checklist of concrete, code-level steps)

- [x] `pub fn park_lens(&mut self, slot_id: SlotId, now: Ts) -> Result<LifecycleOutcome>`:
  1. Look up slot; if absent → core lifecycle fail-closed error.
  2. If `Retired` → core lifecycle fail-closed error (cannot park a tombstone;
     use descriptive remediation: "lens is retired; park is only valid for
     active or previously-parked lenses").
  3. If already `Parked` → no-op, `Ok(())`.
  4. `panel.slots[index].state = SlotState::Parked`.
  5. `panel.version += 1` only for the state change.
  6. Cancel pending backfill for this slot (do not waste resources).
- [x] `pub fn unpark_lens(&mut self, slot_id: SlotId, now: Ts) -> Result<LifecycleOutcome>`:
  1. Look up slot; if absent or `Retired` → core lifecycle fail-closed error.
  2. If already `Active` → no-op, `Ok(())`.
  3. `panel.slots[index].state = SlotState::Active`.
  4. `panel.version += 1` only for the state change.
- [x] `SlotIndexMap` checks `SlotState::Parked` / `Retired` for search and
  insert paths and returns `CALYX_SEXTANT_SLOT_INACTIVE` (#327).

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: `add_lens` → `park_lens` → `slot_states[slot_id] == Parked`,
  `panel_version == 2`.
- [x] unit: `park_lens` already-parked → no-op, `panel_version` unchanged.
- [x] unit: `park_lens`/`unpark_lens` on retired slot →
  `CALYX_LENS_FROZEN_VIOLATION` in the current core catalog (#327).
- [x] unit: `park_lens` then `unpark_lens` → `slot.state == Active`,
  `panel_version == 3`; queue mutation is not fabricated on unpark.
- [x] unit: `unpark_lens` already-active slot → no-op, `panel_version`
  unchanged.
- [x] edge (≥3): (1) park → measure returns `LensInactive`; (2) unpark →
  measure returns a real vector; (3) `panel_version` sequence for
  add+park+unpark is strictly 1, 2, 3.
- [x] fail-closed: park on unknown slot → exact core lifecycle error.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `registry.slot_states` + `panel_version` sequence
- **Readback:** `cargo test -p calyx-registry park_unpark -- --nocapture 2>&1`
- **Prove:** output shows state transitions `Active→Parked→Active` and
  `panel_version` sequence `1,2,3`; parked measure returns `LensInactive`;
  unparked measure returns a vector; screenshot attached to PH20 GitHub issue

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] FSV evidence (readback output / screenshot) attached to the PH20 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
