# PH20 · T01 — add_lens: slot allocation + panel_version bump

| Field | Value |
|---|---|
| **Phase** | PH20 — Hot-swap add/retire/park + lazy backfill |
| **Stage** | S3 — Registry / Lenses |
| **Crate** | `calyx-registry` |
| **Files** | `crates/calyx-registry/src/swap.rs` (≤500), `crates/calyx-registry/src/slot_alloc.rs` (≤500) |
| **Depends on** | PH19 (Registry + all runtimes), PH09 (Aster slot CFs) |
| **Axioms** | A5 |
| **PRD** | `dbprdplans/05 §3` |

## Goal

Implement `add_lens(spec) -> LensId` as specified in `05 §3`:
validate the frozen contract, content-address to `LensId`, no-op if already
registered, allocate the next `SlotId`, create an empty slot CF column and
ANN index placeholder, bump `panel_version`, and schedule lazy backfill.
No existing constellation is rewritten.

## Build (checklist of concrete, code-level steps)

- [ ] `pub fn add_lens(registry: &mut Registry, spec: LensSpec, lens: Box<dyn Lens>, store: &dyn VaultStore) -> Result<LensId>`:
  1. `check_frozen_contract_at_register(&spec, lens.as_ref(), &probe_input)`.
  2. `let id = compute_lens_id(&spec)`; if `registry.contains(id)` → `Ok(id)` (idempotent no-op).
  3. `let slot_id = registry.alloc_next_slot_id()`.
  4. `registry.lenses.insert(id, (spec.clone(), lens))`.
  5. `registry.slot_map.insert(slot_id, id)`.
  6. `registry.panel_version += 1`.
  7. Create slot CF placeholder entry in Aster (stub: write a sentinel row
     `slot_{slot_id}/HEADER = SlotState::Active + panel_version` via
     `store`). If `store` unavailable → record in `registry.pending_cf_creates`.
  8. Create empty ANN index placeholder (unit stub — real index in PH23).
  9. Enqueue `BackfillRequest { slot_id, priority: BackfillPriority::Normal }`.
  10. Return `Ok(id)`.
- [ ] `SlotId` allocation: `registry.next_slot_id: SlotId` counter; increment
  atomically; never reuse a retired slot's id within a vault lifetime.
- [ ] `Registry` gains fields: `slot_map: HashMap<SlotId, LensId>`,
  `panel_version: u32`, `next_slot_id: SlotId`, `backfill_queue: VecDeque<BackfillRequest>`.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: `add_lens` on a fresh registry → `panel_version` goes from 0 to 1;
  `slot_map` has one entry; `lenses` has one entry.
- [ ] unit: `add_lens` same spec twice → second call returns same `LensId`;
  `panel_version` stays at 1 (idempotent).
- [ ] unit: `add_lens` two different specs → `panel_version == 2`, two distinct
  `SlotId`s allocated.
- [ ] proptest: `panel_version` after N `add_lens` calls (all unique specs) ==
  N (monotone increment, no skips).
- [ ] edge (≥3): (1) frozen contract violation on registration → no slot
  allocated, `panel_version` unchanged; (2) `slot_id` never wraps below
  previous maximum; (3) `backfill_queue` has one entry per successful add.
- [ ] fail-closed: frozen contract failure → `CALYX_LENS_FROZEN_VIOLATION`,
  no state mutation in `registry`.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `registry.panel_version` and `slot_map` in-memory state; Aster
  `slot_*/HEADER` CF row (if store available)
- **Readback:** `cargo test -p calyx-registry add_lens -- --nocapture 2>&1`
- **Prove:** output shows `panel_version=1 slot_id=0` after first add;
  `panel_version=1` after idempotent second add; screenshot attached to PH20
  GitHub issue

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH20 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
