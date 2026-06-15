# PH51 · T02 — `SlotSet`, `SlotTag`, `CompletionResult` types

| Field | Value |
|---|---|
| **Phase** | PH51 — `complete()` unified primitive |
| **Stage** | S11 — Oracle & AGI Layer |
| **Crate** | `calyx-oracle` |
| **Files** | `crates/calyx-oracle/src/types.rs` (extend, ≤500) |
| **Depends on** | T01 (energy types), PH49 T01 (existing oracle types) |
| **Axioms** | A2, A16 |
| **PRD** | `dbprdplans/26 §3`, `dbprdplans/26 §11.1` |

## Goal

Define the types that `complete()` operates over: `SlotSet` (the set of slot IDs that are
clamped vs free), `SlotTag` (the epistemic status of each filled slot), and `CompletionResult`
(the fully filled constellation with per-slot tags). The invariant: clamped slots always carry
`SlotTag::Measured`; free (filled) slots carry `SlotTag::Inferred` (converged descent) or
`SlotTag::Provisional` (non-converged or near-insufficient). The `SlotTag` discipline is
A2/A16: "completed slots tagged `inferred`/`provisional`, never confused with measured ones."

## Build (checklist of concrete, code-level steps)

- [ ] `enum SlotTag { Measured, Inferred, Provisional }` — `Measured` = original data; `Inferred` = energy-descent converged; `Provisional` = descent not converged or panel near-insufficient
- [ ] `type SlotSet = HashSet<LensId>` — set of lens IDs; semantic alias for clarity
- [ ] `struct TaggedSlot { lens_id: LensId, vector: Vec<f32>, tag: SlotTag }` — one slot in the filled constellation
- [ ] `struct CompletionResult { filled_cx: Vec<TaggedSlot>, confidence: f32, converged: bool, energy: f32, provenance: LedgerRef }` — `confidence ≤ oracle_self_consistency.ceiling`; `converged` mirrors `DescentResult.converged`
- [ ] `impl CompletionResult { fn inferred_slots(&self) -> Vec<&TaggedSlot>` (filter by `SlotTag::Inferred`); `fn provisional_slots(&self) -> Vec<&TaggedSlot>` (filter by `SlotTag::Provisional`); `fn measured_slots(&self) -> Vec<&TaggedSlot>` }
- [ ] Invariant enforced in constructor: `clamp ∩ free = ∅`; `clamp ∪ free = all_slots`; violation → `OracleError` with `CALYX_ORACLE_SLOT_CONFLICT` code
- [ ] Add `CALYX_ORACLE_SLOT_CONFLICT` to the error catalog in `src/error.rs`
- [ ] All types `#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]`; serde round-trip for `SlotTag` encodes as string `"measured"` / `"inferred"` / `"provisional"` (human-readable JSON)

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: `CompletionResult` with 3 measured + 4 inferred slots; `inferred_slots().len() == 4`; `measured_slots().len() == 3`
- [ ] unit: `SlotTag` serde round-trip: `"inferred"` → `SlotTag::Inferred` → `"inferred"` byte-identical
- [ ] unit: `clamp ∩ free` non-empty → `CALYX_ORACLE_SLOT_CONFLICT` returned
- [ ] proptest: `inferred_slots().len() + provisional_slots().len() + measured_slots().len() == filled_cx.len()`
- [ ] edge (≥3): all slots clamped (free = empty) → `inferred_slots()` empty; all slots free (clamp = empty) → `measured_slots()` empty; zero slots total → `CompletionResult` with empty `filled_cx`
- [ ] fail-closed: `CALYX_ORACLE_SLOT_CONFLICT` contains remediation text; `clamp ∪ free ≠ all_slots` → error with the missing slot IDs listed

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `crates/calyx-oracle/src/types.rs`; test output showing `SlotTag` serde encoding
- **Readback:** `cargo test -p calyx-oracle -- slot_tag --nocapture`; `grep -c "inferred\|provisional\|measured" output` shows ≥3 distinct encodings
- **Prove:** serde round-trip byte-identical for all three `SlotTag` variants; `inferred_slots()` count correct; `CALYX_ORACLE_SLOT_CONFLICT` fires on overlap

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH51 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
