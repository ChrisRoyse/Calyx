# PH30 — Panel sufficiency + attribution + reports

**Stage:** S5 — Loom + Assay (DDA & Bits)  ·  **Crate:** `calyx-assay` / `calyx-loom`  ·
**PRD roadmap:** A8  ·  **Axioms:** A2, A8

## Objective

Implement the substrate-sufficiency test — `I(panel; anchor)` vs `H(anchor)` —
and the per-sensor bit-attribution decomposition. Complete the `abundance_report`
with the DPI ceiling and `bits_report` with per-slot marginal bits and sole-carrier
flags. A known-insufficient panel (`I ≪ H`) must be flagged with the per-slot
deficit and routes to Anneal's lens-proposal path (PH47). Trusted bits only when
grounded; else `provisional` (A2). The honest dashboard closes Stage 5.

> **DPI ceiling is load-bearing (A8):** `abundance_report` MUST expose
> `I(panel; anchor)` as the ceiling alongside `C(N,2)`. Selling `C(N,2)` without
> the ceiling is forbidden. The four honest numbers — N, C(N,2), materialized,
> n_eff, DPI ceiling — must all be present and non-fabricated.

## Dependencies

- **Phases:** PH29 (admit_lens, n_eff, stratified bits, assay CF), PH28 (KSG
  MI estimators, bootstrap CI), PH27 (abundance report skeleton, xterm CF,
  agreement_graph)
- **Provides for:** PH47 (Anneal lens-proposal reads sufficiency deficit),
  PH31 (Lodestar kernel-graph trustworthiness depends on panel sufficiency),
  PH48 (J objective reads bits_report)

## Current state (build off what exists)

`calyx-assay` now has MI estimators, logistic-probe lens/pair signal,
`AssayGate`, differentiation gates, stratified bits, stable-rank n_eff,
panel sufficiency, attribution, `bits_report`, and an assay cache/store with
provenance. `calyx-loom` now has `AbundanceReport` with computed n_eff and DPI
ceiling fields. Stage 5 FSV readback is the JSON source-of-truth emitted by
`stage5_full_stack_fsv`; user-facing `calyx abundance` and `calyx bits-report`
commands are deferred to the Stage 18 CLI surface (PH62), not required for
Lodestar.

Post-sweep #291 requires persisted Assay CF rows to carry explicit vault scope
and anchor scope. The FSV readback records `all_rows_scoped`, `vault_scope`, and
`anchor_scope`, plus malformed-estimator edge codes for short, ragged, and
non-finite samples.

## Deliverables (file plan, each ≤500 lines)

| File | Responsibility |
|---|---|
| `crates/calyx-assay/src/sufficiency.rs` | `panel_sufficiency(anchor) -> { I_panel, H_anchor, deficit, per_slot_attribution }`; routes deficit to Anneal |
| `crates/calyx-assay/src/attribution.rs` | Per-slot marginal bits: `I(panel;anchor) − I(panel∖k;anchor)`; sole-carrier flag; `bits_report` |
| `crates/calyx-assay/src/samples.rs` | Shared finite rectangular sample-matrix guard for Assay estimators |
| `crates/calyx-loom/src/abundance.rs` | Complete `abundance_report` with real n_eff + DPI ceiling (replaces PH27 stubs); `meaning_compression_yield` |
| `crates/calyx-assay/src/tests.rs` | Planted-insufficient panel FSV; per-slot deficit FSV; trusted vs provisional tagging |

## Tasks (atomic — all must pass for the phase to be DONE)

| Card | Title | Depends |
|---|---|---|
| T01 | `panel_sufficiency`: `I(panel;anchor)` vs `H(anchor)` | — |
| T02 | Per-sensor attribution: marginal bits + sole-carrier flag | T01 |
| T03 | `bits_report` + complete `abundance_report` with DPI ceiling | T01, T02 |
| T04 | Planted-insufficient panel FSV + trusted/provisional tagging | T01, T02, T03 |
| T05 | Deficit routing to Anneal + Stage 5 exit gate | T04 |

## FSV exit gate (the phase is DONE only when this is byte-proven on aiwonder)

1. **`abundance_report` emits the four honest numbers:**
   ```
   CALYX_FSV_ROOT=/home/croyse/calyx/data/fsv-stage5-loom-assay-20260608-final \
     cargo test -p calyx-assay stage5_full_stack_fsv -- --ignored --nocapture
   ```
   Output must contain: N (integer), C(N,2) (= N*(N-1)/2), materialized (count),
   n_eff (Computed, not [provisional]), DPI ceiling (I(panel;anchor) in bits,
   Computed). The byte readback lives in `stage5-readback.json`.

2. **Known-insufficient panel flagged with per-slot deficit:**
   ```
   cargo test panel_insufficiency_planted -- --nocapture
   ```
   A panel with `I(panel;anchor) ≈ 0.46 bits` where `H(anchor) ≈ 1.0 bit` must
   flag deficit ≈ 0.54 bits; the per-slot attribution must identify the slot(s)
   carrying the least bits.

3. **Trusted bits only when grounded:**
   ```
   cargo test bits_trust_grounded_vs_provisional -- --nocapture
   ```
   Bits against a grounded anchor → `Trusted`; bits against an auto-labeled
   anchor → `Provisional`; the distinction is byte-readable in the assay CF.

Evidence (all three readbacks) attached to PH30 GitHub issue.

## Risks / landmines

- **`H(anchor)` estimation:** entropy of the anchor distribution must be
  estimated from the same labeled samples as the MI — not from a hard-coded
  theoretical value. For binary anchors, `H = -p·log2(p) - (1-p)·log2(1-p)`;
  for multi-class, use frequency counts. Attach CI from bootstrap.
- **Panel MI via chain rule:** `I(panel; anchor)` for high-N panels is computed
  via the chain rule `I(panel;Y) = Σ I(slot_k; Y | slot_{k-1},…,slot_1)` or
  via the joint KSG on the concatenated panel vector. The concatenated-vector
  route degrades at high-d; use the chain rule or random-project the panel
  first. Document the choice.
- **Sole-carrier detection:** a slot is a sole carrier if removing it drops
  `I(panel;anchor)` by more than its own marginal bits (i.e., some other slot
  was covering for it). Use the marginal-value formula; do not re-run full
  leave-one-out KSG unless N ≤ 5 (expensive for large N).
- **Anneal routing:** the deficit output must be a structured `SufficiencyDeficit`
  with `slot_id`, `missing_bits`, and `suggested_anchor` fields that PH47 can
  consume. Do not route via a log message.
