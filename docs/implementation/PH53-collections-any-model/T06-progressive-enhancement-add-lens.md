# PH53 · T06 — Progressive enhancement: 0-lens = plain store; `add_lens` upgrades to Constellations

| Field | Value |
|---|---|
| **Phase** | PH53 — Collections-as-any-model (relational/doc/KV/TS/blob) |
| **Stage** | S12 — Universal data layer |
| **Crate** | `calyx-aster` |
| **Files** | `crates/calyx-aster/src/collection/mod.rs` (amend T01 file, ≤500 total) |
| **Depends on** | T01, T02, T03, T04, T05, PH09 (Constellation CRUD), PH20 (lens backfill — PH53 stubs only) |
| **Axioms** | A5, A15, A16, A19 |
| **PRD** | `dbprdplans/03 §0`, `dbprdplans/20 §3` |

## Goal

Implement and test the progressive-enhancement contract: a collection with 0
lenses (`panel=None`) is a plain fast store and the intelligence stack is not
invoked at all; calling `add_lens` on it sets `panel` and mode to
`Constellations`, triggering lazy backfill of existing records (PH20 does the
actual backfill; PH53 sets the flag and validates the state machine). This is
the killer property of `20 §3`: intelligence is opt-in, one `add_lens` call.

## Build (checklist of concrete, code-level steps)

- [ ] Enforce in `put_record` / `kv_set` / `ts_write` / `blob_put`:
  if `col.panel.is_some()` → call `AsterVault::ingest_constellation` (route
  to Constellation CRUD from PH09); else → call the plain-layer `put` (T02–T05).
  This is the dispatch fork that makes "0-lens = plain store" true.
- [ ] Implement `add_lens(vault: &AsterVault, collection_name: &str, lens_id: LensId) -> Result<()>`:
  - Load `Collection` metadata from `collections` CF.
  - Validate: `mode != Constellations` (not already upgraded); `lens_id` must
    exist in the registry (or fail with `CALYX_LENS_NOT_FOUND`).
  - Set `col.panel = Some(PanelRef::new(lens_id))`, `col.mode = Constellations`.
  - Write updated `Collection` back to `collections` CF in WAL batch.
  - Write a `backfill_pending` marker row in the `online` CF (key =
    `b"backfill\x00" ++ collection_id`); PH20 consumes this to run backfill.
  - Return `Ok(())` immediately (backfill is async / lazy per A5).
- [ ] Enforce: once `mode = Constellations`, calling `add_lens` a second time
  with the same `lens_id` → `CALYX_COLLECTION_LENS_DUPLICATE` (idempotent
  if called from PH20 backfill controller).
- [ ] Expose `collection_has_lens(col: &Collection) -> bool` as a cheap inline
  check used by the write dispatch.
- [ ] Confirm with a test: a plain `Records` collection write goes through
  `relational.rs` and does NOT touch the Constellation CRUD path (no ANN insert,
  no slot column written) — verify by asserting the `slot_00` CF does not grow.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit plain store: create `Records` collection, `put_record` 3 rows →
  `slot_00` CF byte count = 0; `cf/relational/` byte count > 0.
- [ ] unit `add_lens`: create `Records` collection → `add_lens(lex_lens_id)` →
  `get_collection` returns `mode=Constellations`, `panel=Some(…)`;
  `backfill_pending` marker exists in `online` CF.
- [ ] unit post-upgrade: after `add_lens`, `put_record` routes through
  Constellation CRUD path (assert `slot_00` CF grows by 1 row after next write).
- [ ] edge (≥3): (1) `add_lens` on already-Constellations collection →
  `CALYX_COLLECTION_LENS_DUPLICATE`; (2) `add_lens` with unknown `lens_id` →
  `CALYX_LENS_NOT_FOUND`; (3) `add_lens` on `KV` collection → succeeds and
  sets `mode=Constellations`.
- [ ] fail-closed: crash between `add_lens` CF write and `backfill_pending`
  marker write (simulated by injecting `Err` after step 1) → `get_collection`
  still returns old `mode=Records` (WAL replay restores pre-`add_lens` state).

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `collections` CF row (mode field) + `online` CF `backfill_pending` marker.
- **Readback:**
  ```
  calyx collection create --vault /home/croyse/calyx/test-vault --name orders --mode records
  calyx record put --vault /home/croyse/calyx/test-vault --collection orders --pk 1 --data '{"x":1}'
  # confirm no slot CF written:
  wc -c /home/croyse/calyx/test-vault/cf/slot_00/000001.sst 2>&1 || echo "absent"
  # upgrade:
  calyx collection add-lens --vault /home/croyse/calyx/test-vault --collection orders --lens sem-self
  calyx readback --cf collections --vault /home/croyse/calyx/test-vault
  calyx readback --cf online --vault /home/croyse/calyx/test-vault | grep backfill
  ```
- **Prove:** Before `add_lens`: `slot_00` CF absent or 0 bytes; after
  `add_lens`: `get_collection` returns `mode=Constellations`; `backfill_pending`
  marker present. Evidence posted to PH53 issue.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH53 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
