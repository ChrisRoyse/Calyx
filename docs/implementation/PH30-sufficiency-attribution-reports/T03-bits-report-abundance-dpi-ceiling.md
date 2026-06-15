# PH30 · T03 — `bits_report` + complete `abundance_report` with DPI ceiling

> **Status: DONE in Stage 5 core.** `crates/calyx-loom/src/abundance.rs` and
> `crates/calyx-assay/src/attribution.rs` implement the report structs and the
> Stage 5 FSV readback writes `stage5-readback.json` under the FSV root. The
> standalone `calyx abundance` and `calyx bits-report` UX commands are deferred
> to PH62, where the CLI surface is built.

| Field | Value |
|---|---|
| **Phase** | PH30 — Panel sufficiency + attribution + reports |
| **Stage** | S5 — Loom + Assay (DDA & Bits) |
| **Crate** | `calyx-assay`, `calyx-loom` |
| **Files** | `crates/calyx-loom/src/abundance.rs` (≤500), `crates/calyx-assay/src/attribution.rs` (≤500) |
| **Depends on** | T01 (PanelSufficiency), T02 (SlotAttribution), PH29 T03 (n_eff) |
| **Axioms** | A2, A8 |
| **PRD** | `dbprdplans/07 §4`, `07 §5`, `06 §8`, `06 §2` |

## Goal

Complete the `abundance_report` — replace the PH27 stubs (`n_eff: Provisional`,
`dpi_ceiling: Provisional`) with real computed values. Implement `bits_report` —
the per-slot bit-attribution table. Both reports are the honest dashboards that
make DDA truthful: N, C(N,2), materialized, n_eff, DPI ceiling must all be
present, real, and non-fabricated. This closes the reporting layer of Stage 5.

## Build (checklist of concrete, code-level steps)

- [x] Update `AbundanceReport` in `calyx-loom/src/abundance.rs`:
  - replace `n_eff: NeffEstimate::Provisional(N)` with the real value from `n_eff_panel` (PH29 T03)
  - replace `dpi_ceiling_bits: DpiCeiling::Provisional` with `DpiCeiling::Computed { bits: i_panel, anchor: AnchorKind }` from `panel_sufficiency` (T01)
  - add `sufficiency_verdict: Sufficient | Insufficient { deficit_bits: f32 }` field
  - add `meaning_compression_yield: f32` = `materialized_xterms as f32 / n_constellations as f32` (signals materialized per real input; `NaN` when `n_constellations = 0`)
- [x] Define `BitsReport`:
  ```rust
  pub struct BitsReport {
      pub panel_id: PanelId,
      pub anchor: AnchorKind,
      pub i_panel: MiEstimate,
      pub h_anchor: EntropyEstimate,
      pub slot_attributions: Vec<SlotAttribution>,
      pub n_eff: NeffEstimate,
      pub computed_at_seq: u64,
      pub trust: Trusted | Provisional,
  }
  ```
- [x] Implement `bits_report(panel, anchor, vault, forge, clock) -> Result<BitsReport, CalyxError>`:
  - calls `panel_sufficiency` (T01) and `attribute_panel` (T02)
  - assembles `BitsReport`; tags `trust: Trusted` iff anchor is grounded (A2)
  - persists to the assay CF: keyed `(panel_id, anchor_kind, shard_hash, ts)`
- [x] `Display` impl for both `AbundanceReport` and `BitsReport` that prints all fields; marks `[provisional]` only when trust is `Provisional`; never hides `C(N,2)` or the DPI ceiling
- [x] Core report integration: `stage5_full_stack_fsv` writes `AbundanceReport`
  and `BitsReport` JSON readbacks under the Stage 5 FSV root.
- [x] CLI integration deferred to PH62: `calyx abundance --vault <path>` prints
  `AbundanceReport`; `calyx bits-report --panel <id> --anchor <kind>` prints
  `BitsReport`.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: `abundance_report` for a test vault with N=13, n_eff=4.2 (computed), DPI ceiling=0.7 bits → all four honest numbers printed; `[provisional]` absent from n_eff and DPI ceiling fields
- [x] unit: `bits_report` for a planted panel: 3 slots, sole carrier on slot_a → report shows `slot_a.sole_carrier: true`, `i_panel ≈ slot_a.individual_bits` (no other slot carries signal)
- [x] proptest: `abundance_report.dpi_ceiling_bits ≤ abundance_report.cn2_upper_bound * n_constellations` is never printed as a bare integer without context (formatting invariant — the ceiling is in bits, not a raw xterm count)
- [x] edge: vault with no grounded anchors → `dpi_ceiling_bits: Provisional` (no grounded anchor to compute against); `trust: Provisional` throughout; report still printed (not suppressed)
- [x] fail-closed: assay CF write failure → `CALYX_ASTER_IO` propagated; report is never partially persisted (all-or-nothing write)

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** the printed `abundance_report` for a test vault on aiwonder with N=13 lenses, real n_eff, and a grounded anchor
- **Readback:**
  ```
  CALYX_FSV_ROOT=/home/croyse/calyx/data/fsv-stage5-loom-assay-20260608-final \
    cargo test -p calyx-assay stage5_full_stack_fsv -- --ignored --nocapture
  ```
  Expected output format (exact fields required):
  ```
  N (active lenses):        13
  C(N,2) upper bound:       78
  Materialized xterms:      <count>
  n_eff:                    <f32>   (not [provisional])
  DPI ceiling:              <f32> bits  (I(panel;anchor), not [provisional])
  Sufficiency verdict:      Sufficient | Insufficient (deficit: <f32> bits)
  Meaning compression:      <f32> signals/input
  ```
- **Prove:** run on aiwonder; read `stage5-readback.json`,
  `xterm-cf-readback.json`, and `assay-cf-readback.json`; confirm all honest
  numbers are present and not fabricated. Post evidence to PH30 GitHub issue.

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] FSV evidence (readback output / screenshot) attached to the PH30 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
