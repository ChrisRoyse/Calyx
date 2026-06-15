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

Implementation note after #311: `SwapController::add_lens` remains the
in-memory queue path for unit-level callers. Production/manual FSV uses
`SwapController::add_lens_durable`, which performs the same panel mutation and
persists a `BackfillScheduler` request in the same API call. If scheduler
enqueue fails, the controller and scheduler objects are restored to their
pre-call state before the error is returned.

Post-sweep #314: both add paths now require `&Registry` and verify the requested
`LensId` has a frozen registered contract with matching slot shape/modality
before any panel version, queue, or scheduler mutation. Unregistered or unfrozen
lenses fail closed with `CALYX_LENS_FROZEN_VIOLATION`.

Post-sweep #327: an identical live slot add is idempotent. It returns the
existing slot, leaves `panel_version` unchanged, marks the index placeholder
ready, and enqueues no duplicate backfill. Reusing the same live lens under a
different slot key still fails closed.

## Build (checklist of concrete, code-level steps)

- [x] `pub fn add_lens_durable(controller, registry, spec, candidates, now, scheduler, priority) -> Result<AddLensOutcome>`:
  1. `registry.frozen_contract(spec.lens_id)` must exist before mutation.
  2. The frozen contract shape/modality must match `SlotSpec`.
  3. `let slot_id = registry.alloc_next_slot_id()`.
  4. `registry.lenses.insert(id, (spec.clone(), lens))`.
  5. `registry.slot_map.insert(slot_id, id)`.
  6. `registry.panel_version += 1`.
  7. Create slot CF placeholder entry in Aster (stub: write a sentinel row
     `slot_{slot_id}/HEADER = SlotState::Active + panel_version` via
     `store`). If `store` unavailable → record in `registry.pending_cf_creates`.
  8. Create empty ANN index placeholder (unit stub — real index in PH23).
  9. Enqueue persisted `BackfillRequest { slot_id, lens_id, priority, candidates }`.
  10. Return `Ok(id)`.
- [x] `SwapController` allocates `SlotId` from the current panel max and never
  reuses a retired slot id within the panel lifetime.
- [x] `SwapController` owns `Panel.version` plus the in-memory `BackfillQueue`;
  `add_lens_durable` also persists the `BackfillScheduler` request.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: `add_lens` on a fresh registry → `panel_version` goes from 0 to 1;
  `slot_map` has one entry; `lenses` has one entry.
- [x] unit: `add_lens` same spec twice → second call returns the same slot,
  `panel_version` is unchanged, and no duplicate backfill is queued (#327).
- [x] unit: `add_lens` two different specs → `panel_version == 2`, two distinct
  `SlotId`s allocated.
- [x] proptest: `panel_version` after N `add_lens` calls (all unique specs) ==
  N (monotone increment, no skips).
- [x] edge (≥3): (1) frozen contract violation on registration → no slot
  allocated, `panel_version` unchanged; (2) `slot_id` never wraps below
  previous maximum; (3) `backfill_queue` has one entry per successful add.
- [x] fail-closed: missing frozen contract → `CALYX_LENS_FROZEN_VIOLATION`,
  no panel, queue, or durable scheduler mutation (#314).

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `SwapController.panel`, in-memory backfill queue, durable
  `backfill-watermark.json`, and Aster slot CF rows for the full PH20 FSV
- **Readback:** `cargo test -p calyx-registry add_lens -- --nocapture 2>&1`
- **Prove:** output shows `panel_version=1 slot_id=0` after first add;
  `panel_version=1` after idempotent second add; screenshot attached to PH20
  GitHub issue

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] FSV evidence (readback output / screenshot) attached to the PH20 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
