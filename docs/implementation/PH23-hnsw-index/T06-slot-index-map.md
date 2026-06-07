# PH23 · T06 — `SlotIndexMap` concurrent-read-safe registry

| Field | Value |
|---|---|
| **Phase** | PH23 — Per-slot HNSW index |
| **Stage** | S4 — Sextant Search & Navigation |
| **Crate** | `calyx-sextant` |
| **Files** | `crates/calyx-sextant/src/slot_index_map.rs` (≤500) |
| **Depends on** | T05 (this phase) · PH20 (`SlotId`) |
| **Axioms** | A16, A26 |
| **PRD** | `dbprdplans/10 §3` |

## Goal

`SlotIndexMap` is the `SlotId → Box<dyn Index>` registry that PH24 fusion will
call. It must be concurrent-read-safe (many simultaneous search calls across
lenses) and fail-closed on missing slots. Per-slot cost isolation: a search that
specifies two slots only touches those two indexes.

## Build (checklist of concrete, code-level steps)

- [ ] `SlotIndexMap` struct backed by `DashMap<SlotId, RwLock<Box<dyn Index>>>`
      or `parking_lot::RwLock<HashMap<SlotId, Box<dyn Index>>>` (choose and
      document; the latter is simpler and preferred for embedded use)
- [ ] `fn register(&mut self, slot: SlotId, index: Box<dyn Index>) -> Result<(), CalyxError>`:
      fail if slot already registered with a different dim →
      `CALYX_SEXTANT_SLOT_ALREADY_REGISTERED`
- [ ] `fn insert(&self, slot: SlotId, id: CxId, vec: &[f32]) -> Result<(), CalyxError>`:
      acquires write lock for the slot; `CALYX_SEXTANT_SLOT_NOT_FOUND` if absent
- [ ] `fn search(&self, slot: SlotId, query: &[f32], k: usize, ef: usize) -> Result<Vec<(CxId, f32)>, CalyxError>`:
      acquires read lock; `CALYX_SEXTANT_SLOT_NOT_FOUND` if absent
- [ ] `fn slots(&self) -> Vec<SlotId>` — lists registered slots (for planner)
- [ ] `fn rebuild_slot(&self, slot: SlotId) -> Result<(), CalyxError>`:
      acquires write lock, calls `index.rebuild()`; used by Anneal self-heal (PH44)

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: register two slots, insert 5 vecs each, search each slot → results
      are independent (no cross-slot contamination)
- [ ] unit: `slots()` returns both registered slots, in deterministic order
- [ ] proptest: concurrent reads from N threads (N=4) on the same slot all succeed
      and return identical results for the same query
- [ ] edge: `insert` to unregistered slot → `CALYX_SEXTANT_SLOT_NOT_FOUND`
- [ ] edge: `register` same slot twice with different dim →
      `CALYX_SEXTANT_SLOT_ALREADY_REGISTERED`
- [ ] edge: `search` after `rebuild_slot` returns same results as before rebuild
      (recall@5 == 1.0 on small set)
- [ ] fail-closed: `search` on empty map → `CALYX_SEXTANT_SLOT_NOT_FOUND`

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** test output of `cargo test -p calyx-sextant slot_index_map -- --nocapture`
- **Readback:** `cargo test -p calyx-sextant slot_index_map -- --nocapture 2>&1`
- **Prove:** concurrent-read test prints `threads=4 all_ok=true`; slot isolation
  test prints `slot_a_results ≠ slot_b_results` (different random vecs inserted)

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH23 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
