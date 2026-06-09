# PH30 · T02 — Per-sensor attribution: marginal bits + sole-carrier flag

| Field | Value |
|---|---|
| **Phase** | PH30 — Panel sufficiency + attribution + reports |
| **Stage** | S5 — Loom + Assay (DDA & Bits) |
| **Crate** | `calyx-assay` |
| **Files** | `crates/calyx-assay/src/attribution.rs` (≤500) |
| **Depends on** | T01 (PanelSufficiency, panel_mi) · PH28 T06 (lens_signal) |
| **Axioms** | A2, A8 |
| **PRD** | `dbprdplans/07 §5`, `07 §4` |

## Goal

Implement the bit-attribution table: for each slot, compute marginal bits
`I(panel;anchor) − I(panel∖k;anchor)` (bits lost if this slot is removed) and
flag sole carriers (slots where the signal survives only in that slot, matching
the ME-JEPA "no sensor is load-bearing" diagnostic). This is the actionable
artifact: an agent reads it and knows exactly which lens to add or cut.

## Build (checklist of concrete, code-level steps)

- [x] Define `SlotAttribution`:
  ```rust
  pub struct SlotAttribution {
      pub slot_id: SlotId,
      pub marginal_bits: f32,          // I(panel;Y) - I(panel∖k;Y)
      pub individual_bits: f32,         // I(slot_k;Y) standalone
      pub redundancy_fraction: f32,     // how much of its signal is already in others
      pub sole_carrier: bool,           // true iff removing this slot drops I by more than individual_bits
      pub trust: Trusted | Provisional,
  }
  ```
- [x] Implement `slot_marginal_bits(slot_k: SlotId, panel: &Panel, anchor_labels: &[u32], panel_mi_total: f32, vault, forge, clock) -> Result<f32, CalyxError>`:
  - compute `I(panel∖k; anchor)` by re-running `panel_mi` on the panel minus `slot_k`
  - `marginal = panel_mi_total − I(panel∖k; anchor)`
  - for N > 8: use the approximation `marginal ≈ I(slot_k; anchor) − Σ_{j≠k} pairwise_redundancy_nmi(slot_k, slot_j) / (N-1)` (cheaper; documented)
- [x] Implement `attribute_panel(panel, anchor_labels, panel_mi_total, vault, forge, clock) -> Result<Vec<SlotAttribution>, CalyxError>`:
  - for each active slot `k`: call `slot_marginal_bits`, `lens_signal` (individual bits), compute redundancy fraction
  - set `sole_carrier = marginal_bits > individual_bits * 0.9` (slot carries signal not captured by others)
  - tag `trust: Trusted` iff anchor is grounded (A2)
- [x] Wire `attribute_panel` into `PanelSufficiency.per_slot_attribution` (fill the empty vec from T01)
- [x] `marginal_value(slot_k, anchor) -> Result<f32, CalyxError>`: public API matching `07 §8`

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: panel of 3 slots where slot_a carries all the signal, slot_b and slot_c are noise → `slot_a.sole_carrier = true`, `slot_a.marginal_bits ≈ I(panel;anchor)`, `slot_b.marginal_bits ≈ 0.0`, `slot_c.marginal_bits ≈ 0.0`
- [x] unit: panel of 3 slots with equal signal (each carries 1/3 of the bits) → no sole carriers; all `marginal_bits ≈ I_total/3`
- [x] proptest: `Σ marginal_bits ≤ panel_mi_total * 1.1` (attributions don't exceed total MI by more than 10% due to approximation)
- [x] edge: panel with 1 slot → that slot is trivially the sole carrier; marginal_bits = I(panel;anchor); panel with 0 slots → empty attribution vec
- [x] fail-closed: slot missing from Aster → `CALYX_ASTER_NOT_FOUND` propagated; never returns a fabricated 0.0 marginal for missing data

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** per-slot attribution table for a planted panel where slot_a is the sole carrier (slot_b and slot_c are random noise lenses)
- **Readback:**
  ```
  cargo test per_sensor_sole_carrier_planted -- --nocapture
  ```
  Output: slot_a `sole_carrier: true`, slot_b `marginal_bits ≈ 0.0`, slot_c `marginal_bits ≈ 0.0`.
- **Prove:** run on aiwonder; confirm sole_carrier flag fires on slot_a only. Also confirm that removing slot_a from the panel (calling `panel_mi` without it) drops MI to ≈ 0.0 (verifies the marginal bits calculation is real, not a guess).

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] FSV evidence (readback output / screenshot) attached to the PH30 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
