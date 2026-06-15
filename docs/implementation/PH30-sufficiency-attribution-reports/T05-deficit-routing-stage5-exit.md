# PH30 · T05 — Deficit routing to Anneal + Stage 5 exit gate

> **Status: DONE in Stage 5 core.** `calyx-assay` emits structured
> sufficiency deficits to `SufficiencyDeficitSink`; the Stage 5 exit evidence is
> the ignored `stage5_full_stack_fsv` readback under the aiwonder FSV root.
> Anneal consumes this interface in PH47, and the human CLI commands are
> deferred to PH62.

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
can act on the sufficiency gap without parsing text. Stage 5 now emits deficits
through `SufficiencyDeficitSink`; Anneal consumption is PH47 and the user-facing
CLI is PH62. Verify the complete Stage 5 exit gate: Calyx knows, in bits, what
every lens is worth and whether the panel can answer the question, with DPI
ceiling reported and the differentiation contract gated. This is the final card
of Stage 5.

## Build (checklist of concrete, code-level steps)

- [x] Define `SufficiencyDeficit`:
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
- [x] Update `panel_sufficiency` to return `Option<SufficiencyDeficit>` when verdict is `Insufficient`:
  - populate `per_slot_gaps` from the attribution table (slots sorted by `individual_bits` ascending = the weakest slots first)
  - `suggested_action: LensProposal` iff there exist outcomes with grounded anchors; `InsufficientData` iff n < 50 labeled samples; `DeepGrounding` iff the anchor itself is `Provisional`
- [x] Implement the `SufficiencyDeficitSink` trait: `fn receive_deficit(&self, deficit: SufficiencyDeficit)` — the interface PH47 (Anneal) implements; stub impl in this crate just logs the deficit to the Ledger
- [x] Wire the stub `SufficiencyDeficitSink` into `panel_sufficiency` so the deficit is emitted to the sink (not just returned)
- [x] Stage 5 exit integration test `test_stage5_dda_bits_done`:
  - run `weave` + `ksg_with_ci` + `admit_lens` + `panel_sufficiency` + `bits_report` + `abundance_report` on a single vault end-to-end (seeded synthetic, N=5, 100 constellations, 100 grounded labels)
  - assert: agreement scalars computed; lazy xterm on demand; admission decision made; n_eff computed; bits_report generated; abundance_report has all four honest numbers; no `[provisional]` where grounded

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: `SufficiencyDeficit` for a 3-slot panel where slot_c is the weakest → `per_slot_gaps[0].slot_id == slot_c` (worst first); `suggested_action: LensProposal`
- [x] unit: sink receives deficit when `panel_sufficiency` finds `Insufficient` panel; does not receive anything for `Sufficient` panel
- [x] integration: `test_stage5_dda_bits_done` passes end-to-end on aiwonder (all Stage 5 components wired)
- [x] proptest: deficit `per_slot_gaps` sum of `missing_bits` ≤ `deficit_bits * 1.1` (attributable gap does not exceed total gap by more than 10%)
- [x] edge: all-sufficient panel → `SufficiencyDeficit` not emitted to sink; single-slot panel → one gap entry or none

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** the Stage 5 end-to-end test on aiwonder plus the final
  `stage5-readback.json`, `xterm-cf-readback.json`, and
  `assay-cf-readback.json` files under the FSV root.
- **Readback:**
  ```
  CALYX_FSV_ROOT=/home/croyse/calyx/data/fsv-stage5-loom-assay-20260608-final \
    cargo test -p calyx-assay stage5_full_stack_fsv -- --ignored --nocapture
  cat /home/croyse/calyx/data/fsv-stage5-loom-assay-20260608-final/stage5-readback.json
  ```
- **Prove:**
  1. `stage5_full_stack_fsv` passes on aiwonder (no failures or panics)
  2. `stage5-readback.json` shows:
     - `N`: integer ≥ 1
     - `C(N,2)`: `N*(N-1)/2` exact
     - `Materialized xterms`: integer (Agreement scalars only → ≤ C(N,2) × n_constellations)
     - `n_eff`: `Computed { value: f32 }` (not `[provisional]`)
     - `DPI ceiling`: `Computed { bits: f32 }` (not `[provisional]`)
     - Assay CF readback: `all_rows_scoped=true`, plus explicit `vault_scope`
       and `anchor_scope`
     - Assay estimator edge codes: `insufficient_samples_error`,
       `ragged_samples_error`, and `non_finite_samples_error` all equal
       `CALYX_ASSAY_INSUFFICIENT_SAMPLES`
  3. The `bits_report` JSON shows per-slot attribution with at least one
     `sole_carrier: true` if the planted sole-carrier is in the panel
  4. All evidence posted to PH30 GitHub issue
  5. Stage 5 predicate `DDA_BITS` is satisfied in the BUILD_DONE map (`03 §BUILD_DONE`)

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] FSV evidence attached under
  `/home/croyse/calyx/data/fsv-stage5-loom-assay-20260608-final`
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
