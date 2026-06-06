# T-013 — calyx-core: core structs (Constellation/Slot/…)

**Phase:** PH04 · **Dep:** T-010, T-011, T-012 · **Sudo:** no

## Objective
The constellation data model — the atomic record and its parts — with byte-exact
serde and explicit absence (no zero-fill).

## Preconditions
- T-010 (IDs), T-011 (enums), T-012 (errors).

## Steps
1. `crates/calyx-core/src/model/` (module dir, each file ≤500 lines):
   - `constellation.rs` — `Constellation { cx_id, vault_id, panel_version,
     created_at, input_ref, modality, slots, scalars, anchors, provenance,
     flags }`.
   - `slot.rs` — `Slot { slot_id, slot_key, lens_id, shape, modality, asymmetry,
     quant, axis, bits_about, state, added_at_panel_version }`; `Panel { version,
     slots, created_at, kernel_ref, guard_ref }`.
   - `vector.rs` — `SlotVector { Dense | Sparse | Multi | Absent{reason} }`
     (explicit Absent — A16/A3).
   - `anchor.rs` — `Anchor { kind, value, source, observed_at, confidence }`,
     `AnchorValue`.
   - `signal.rs` — `Signal { bits, ci, n, estimator, ts }`; `CxFlags`;
     `InputRef` (hash + optional pointer, redactable).
   - `mod.rs` facade (explicit `pub use`, no wildcard).
2. serde for all; proptest byte-exact round-trip of a `Constellation`.
3. Test: an `Absent` slot never materializes a zero vector.

## Deliverables
- The `model/` module with all core structs + a `mod.rs` facade + round-trip
  tests.

## FSV gate
`cargo test -p calyx-core` green; serde round-trip of a populated `Constellation`
is **byte-exact** (assert on the encoded bytes); an `Absent` slot stays `Absent`
through a round-trip (no zero-fill); every `model/*.rs` ≤500 lines (gate).

## Done
The constellation model is implemented, round-trips byte-exact, and honors
no-flatten / explicit-absence.

## Refs
PRD `03 §3/§4`, `18 §2`, A1, A3, A16.
