# PH30 · T05 — Deficit routing to Anneal + Stage 5 exit gate

| Field | Value |
|---|---|
| **Phase** | PH30 — Panel sufficiency + attribution + reports |
| **Stage** | S5 — Loom + Assay (DDA & Bits) |
| **Crate** | `calyx-assay`, `calyx-loom` |
| **Files** | `crates/calyx-assay/src/sufficiency.rs` (≤500), `crates/calyx-loom/src/abundance.rs` (≤500) |
| **Depends on** | T04 (planted FSV), T03 (reports), T01 (PanelSufficiency) |
| **Axioms** | A2, A8 |
| **PRD** | `dbprdplans/07 §4`, `06 §8`, `15_STAGE5_LOOM_ASSAY.md` Stage 5 exit |

## Goal

Implement structured deficit routing so downstream consumers (Anneal PH47, CLI)
can act on the sufficiency gap without parsing text. Verify the complete Stage 5
exit gate: Calyx knows, in bits, what every lens is worth and whether the panel
can answer the question, with DPI ceiling reported and the differentiation
contract gated. This is the final card of Stage 5.

## Build (checklist of concrete, code-level steps)

- [ ] Define `SufficiencyDeficit`:
  ```rust
  pub struct SufficiencyDeficit {
      pub panel_id: PanelId,
      pub anchor: AnchorKind,
      pub deficit_bits: f32,
      pub per_slot_gaps: Vec<SlotGap>,          // sorted descending by marginal deficit
      pub suggested_action: LensProposal | DeepGrounding | InsufficientData,
      pub computed_at_seq: u64,
  }
  pub struct SlotGap { pub slot_id: SlotId, pub missing_bits: f32, pub is_sole_carrier_gap: bool }
  ```
- [ ] Update `panel_sufficiency` to return `Option<SufficiencyDeficit>` when verdict is `Insufficient`:
  - populate `per_slot_gaps` from the attribution table (slots sorted by `individual_bits` ascending = the weakest slots first)
  - `suggested_action: LensProposal` iff there exist outcomes with grounded anchors; `InsufficientData` iff n < 50 labeled samples; `DeepGrounding` iff the anchor itself is `Provisional`
- [ ] Implement the `SufficiencyDeficitSink` trait: `fn receive_deficit(&self, deficit: SufficiencyDeficit)` — the interface PH47 (Anneal) implements; stub impl in this crate just logs the deficit to the Ledger
- [ ] Wire the stub `SufficiencyDeficitSink` into `panel_sufficiency` so the deficit is emitted to the sink (not just returned)
- [ ] Stage 5 exit integration test `test_stage5_dda_bits_done`:
  - run `weave` + `ksg_with_ci` + `admit_lens` + `panel_sufficiency` + `bits_report` + `abundance_report` on a single vault end-to-end (seeded synthetic, N=5, 100 constellations, 100 grounded labels)
  - assert: agreement scalars computed; lazy xterm on demand; admission decision made; n_eff computed; bits_report generated; abundance_report has all four honest numbers; no `[provisional]` where grounded

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: `SufficiencyDeficit` for a 3-slot panel where slot_c is the weakest → `per_slot_gaps[0].slot_id == slot_c` (worst first); `suggested_action: LensProposal`
- [ ] unit: sink receives deficit when `panel_sufficiency` finds `Insufficient` panel; does not receive anything for `Sufficient` panel
- [ ] integration: `test_stage5_dda_bits_done` passes end-to-end on aiwonder (all Stage 5 components wired)
- [ ] proptest: deficit `per_slot_gaps` sum of `missing_bits` ≤ `deficit_bits * 1.1` (attributable gap does not exceed total gap by more than 10%)
- [ ] edge: all-sufficient panel → `SufficiencyDeficit` not emitted to sink; single-slot panel → one gap entry or none

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** the Stage 5 end-to-end test on aiwonder + the final `calyx abundance` output + `calyx bits-report` output
- **Readback:**
  ```
  cargo test test_stage5_dda_bits_done -- --nocapture
  calyx abundance --vault /home/croyse/calyx/test-vault
  calyx bits-report --panel default --anchor grounded_outcome
  ```
- **Prove:**
  1. `test_stage5_dda_bits_done` passes on aiwonder (no failures, no panics, no `[provisional]` in reports)
  2. `calyx abundance` shows:
     - `N`: integer ≥ 1
     - `C(N,2)`: `N*(N-1)/2` exact
     - `Materialized xterms`: integer (Agreement scalars only → ≤ C(N,2) × n_constellations)
     - `n_eff`: `Computed { value: f32 }` (not `[provisional]`)
     - `DPI ceiling`: `Computed { bits: f32 }` (not `[provisional]`)
  3. `calyx bits-report` shows per-slot attribution with at least one `sole_carrier: true` if the planted sole-carrier is in the panel
  4. All evidence posted to PH30 GitHub issue
  5. Stage 5 predicate `DDA_BITS` is satisfied in the BUILD_DONE map (`03 §BUILD_DONE`)

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH30 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
