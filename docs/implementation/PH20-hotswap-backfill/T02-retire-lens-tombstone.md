# PH20 · T02 — retire_lens: tombstone + keep history

| Field | Value |
|---|---|
| **Phase** | PH20 — Hot-swap add/retire/park + lazy backfill |
| **Stage** | S3 — Registry / Lenses |
| **Crate** | `calyx-registry` |
| **Files** | `crates/calyx-registry/src/swap.rs` (≤500) |
| **Depends on** | T01 (this phase) |
| **Axioms** | A5 |
| **PRD** | `dbprdplans/05 §3` |

## Goal

Implement `retire_lens(slot_id)` as specified in `05 §3`: mark `SlotState::Retired`
(tombstone), stop measuring the slot on new constellations, stop including it
in searches, but keep its columns and historical vectors for interpretability
until GC policy (PH58) prunes them. Bump `panel_version`. Never delete data
on retire.

Post-sweep #327: `SwapController::retire_lens` is idempotent for an already
retired slot, cancels pending in-memory backfill for the slot on the first
retire, and leaves historical slot rows intact. Sextant now exposes the
search-side inactive-slot gate. The current core error catalog uses
`CALYX_LENS_FROZEN_VIOLATION` for invalid lifecycle transitions; a
registry-specific `CALYX_REGISTRY_LENS_NOT_FOUND` code would be a later catalog
expansion.

## Build (checklist of concrete, code-level steps)

- [x] `pub fn retire_lens(registry: &mut Registry, slot_id: SlotId, store: &dyn VaultStore) -> Result<()>`:
  1. Look up `slot_id` in the panel; if absent →
     core lifecycle fail-closed error.
  2. Look up `LensId` → `LensSpec`; if already `Retired` → no-op, `Ok(())`.
  3. Update `registry.slot_states.insert(slot_id, SlotState::Retired)`.
  4. Do **not** remove the `lens` from `registry.lenses` (keep for historical
     measure calls on old constellations if needed).
  5. Write tombstone to Aster CF: `slot_{slot_id}/HEADER = SlotState::Retired`
     (stub write via `store`).
  6. `registry.panel_version += 1`.
  7. Cancel any pending backfill for this slot.
  8. Return `Ok(())`.
- [x] `SwapController` stores `Slot.state` inside the versioned `Panel`; no
  separate `Registry.slot_states` map is used in the current model.
- [x] `SlotIndexMap` tracks `SlotState`; parked/retired slots are excluded from
  default search and explicit inactive-slot search returns
  `CALYX_SEXTANT_SLOT_INACTIVE` (#327).
- [x] Guard: assert no code path calls `self.slot_states.remove(slot_id)`
  (tombstone is permanent until GC).

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: `add_lens` → `retire_lens` → `slot_states[slot_id] == Retired`;
  `panel_version == 2`.
- [x] unit: `retire_lens` on already-retired slot → `Ok(())`, `panel_version`
  not incremented again (#327).
- [x] unit: `retire_lens` on unknown `slot_id` → `CALYX_LENS_FROZEN_VIOLATION`
  in the current core catalog.
- [x] unit: after `retire_lens`, `Registry::measure` for that slot returns
  `AbsentReason::LensInactive` (not a hard error, not a zero vector).
- [x] unit: the `lenses` map still contains the retired lens entry (history
  preserved).
- [x] edge: pending backfill queue no longer contains active work for the
  retired slot (#327).
- [x] edge (≥3): (1) retire then `add_lens` same spec → new `SlotId` allocated
  (retired id not reused), new slot active; (2) retired slot's CF rows are NOT
  deleted (assert row still present in mock store).
- [x] fail-closed: unknown slot id → exact core lifecycle error.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `registry.slot_states` map + Aster `slot_*/HEADER` CF row showing
  `Retired`; `lenses` map still contains the entry
- **Readback:** `cargo test -p calyx-registry retire_lens -- --nocapture 2>&1`
- **Prove:** test output shows `slot_0 → Retired, panel_version=2`; `lenses`
  map still has one entry; mock store row shows `"retired"` JSON; screenshot
  attached to PH20 GitHub issue

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] FSV evidence (readback output / screenshot) attached to the PH20 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
