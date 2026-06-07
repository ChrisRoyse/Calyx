# PH20 · T05 — No-re-embed invariant + FSV integration test

| Field | Value |
|---|---|
| **Phase** | PH20 — Hot-swap add/retire/park + lazy backfill |
| **Stage** | S3 — Registry / Lenses |
| **Crate** | `calyx-registry` |
| **Files** | `crates/calyx-registry/tests/hotswap_fsv.rs` (≤500) |
| **Depends on** | T01, T02, T03, T04 (this phase) |
| **Axioms** | A5 |
| **PRD** | `dbprdplans/05 §3`, `13_STAGE3_REGISTRY.md §PH20 FSV gate` |

## Goal

Prove the PH20 FSV gate: after `add_lens` on a populated vault, zero existing
constellations are rewritten; the new slot is searchable immediately for new
constellations; backfill fills slot columns over time; and `retire_lens`
tombstones the slot while historical data remains readable.

## Build (checklist of concrete, code-level steps)

- [ ] Test `add_lens_does_not_rewrite_existing_constellations`:
  - build a mock `VaultStore` with N=20 pre-existing constellations (each
    with slot_0 already filled).
  - call `add_lens` → `slot_1` allocated.
  - snapshot all `slot_0` CF rows (their `SlotVector` bytes).
  - assert zero `slot_0` rows changed (byte-for-byte identical).
  - assert `slot_1` rows for all 20 constellations are `AbsentReason::Deferred`.
- [ ] Test `new_slot_searchable_immediately_for_new_cx`:
  - after `add_lens`, ingest a new constellation → its `slot_1` is filled
    immediately (not deferred) because it is a new ingest, not a backfill.
  - assert `slot_1` contains a valid `SlotVector` for the new cx.
- [ ] Test `backfill_fills_slot_columns`:
  - run `BackfillScheduler::tick` enough times to fill all 20 deferred rows.
  - assert zero rows remain `AbsentReason::Deferred` for `slot_1`.
  - print before/after counts: `before: 20 deferred; after: 0 deferred`.
- [ ] Test `retire_tombstones_history_readable`:
  - `retire_lens(slot_0)` → `slot_states[0] == Retired`.
  - read the historical `slot_0` vectors for all 20 constellations from mock
    store → all still present (not deleted).
  - `panel_version` incremented.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] All four sub-tests above pass deterministically with seeded mock data.
- [ ] `add_lens` idempotent: same spec twice → `panel_version == 1`, not 2.
- [ ] After backfill completes, all 20 slot_1 vectors pass `check_output`.
- [ ] edge (≥3): (1) N=0 existing cxs → backfill completes immediately; (2)
  N=100 cxs → backfill progresses in batches of 16; (3) retire after backfill
  completes → historical data intact.
- [ ] fail-closed: any phase that would mutate existing slot columns aborts
  with a panic in debug or `CALYX_REGISTRY_RUNTIME_UNAVAILABLE` in release.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** mock store's CF row map before and after `add_lens` + backfill on
  aiwonder
- **Readback:**
  `cargo test -p calyx-registry hotswap_fsv -- --nocapture 2>&1`
- **Prove:** output shows:
  `slot_0 rows unchanged: 20/20 identical`;
  `slot_1 before backfill: 20 deferred`;
  `slot_1 after backfill: 0 deferred`;
  `retired slot_0: 20 historical rows still present`;
  screenshot attached to PH20 GitHub issue

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH20 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
