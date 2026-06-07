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
operations bump `panel_version`.

## Build (checklist of concrete, code-level steps)

- [ ] `pub fn park_lens(registry: &mut Registry, slot_id: SlotId, store: &dyn VaultStore) -> Result<()>`:
  1. Look up slot; if absent → `CALYX_REGISTRY_LENS_NOT_FOUND`.
  2. If `Retired` → `CALYX_REGISTRY_LENS_NOT_FOUND` (cannot park a tombstone;
     use descriptive remediation: "lens is retired; park is only valid for
     active or previously-parked lenses").
  3. If already `Parked` → no-op, `Ok(())`.
  4. `registry.slot_states.insert(slot_id, SlotState::Parked)`.
  5. Write CF header update (stub via `store`).
  6. `registry.panel_version += 1`.
  7. Cancel pending backfill for this slot (do not waste resources).
- [ ] `pub fn unpark_lens(registry: &mut Registry, slot_id: SlotId, store: &dyn VaultStore) -> Result<()>`:
  1. Look up slot; if absent or `Retired` → `CALYX_REGISTRY_LENS_NOT_FOUND`.
  2. If already `Active` → no-op, `Ok(())`.
  3. `registry.slot_states.insert(slot_id, SlotState::Active)`.
  4. Write CF header update.
  5. `registry.panel_version += 1`.
  6. Enqueue backfill for all constellations added since the slot was parked
     (re-scan watermark: records with `AbsentReason::LensInactive` for this
     slot). Stub: enqueue a full-scan `BackfillRequest` with `priority = Normal`.
- [ ] `Registry::measure` already checks `SlotState::Parked` → returns
  `AbsentReason::LensInactive` (from T02 implementation).

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: `add_lens` → `park_lens` → `slot_states[slot_id] == Parked`,
  `panel_version == 2`.
- [ ] unit: `park_lens` already-parked → no-op, `panel_version` unchanged.
- [ ] unit: `park_lens` on retired slot → `CALYX_REGISTRY_LENS_NOT_FOUND`.
- [ ] unit: `park_lens` then `unpark_lens` → `slot_states == Active`,
  `panel_version == 3`; backfill queue has new entry.
- [ ] unit: `unpark_lens` already-active slot → no-op, `panel_version`
  unchanged.
- [ ] edge (≥3): (1) park → measure returns `LensInactive`; (2) unpark →
  measure returns a real vector; (3) `panel_version` sequence for
  add+park+unpark is strictly 1, 2, 3.
- [ ] fail-closed: park on unknown slot → `CALYX_REGISTRY_LENS_NOT_FOUND`.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `registry.slot_states` + `panel_version` sequence
- **Readback:** `cargo test -p calyx-registry park_unpark -- --nocapture 2>&1`
- **Prove:** output shows state transitions `Active→Parked→Active` and
  `panel_version` sequence `1,2,3`; parked measure returns `LensInactive`;
  unparked measure returns a vector; screenshot attached to PH20 GitHub issue

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH20 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
