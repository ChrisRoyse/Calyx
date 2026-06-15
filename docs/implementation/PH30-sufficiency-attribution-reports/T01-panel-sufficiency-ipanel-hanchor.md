# PH30 · T01 — `panel_sufficiency`: `I(panel;anchor)` vs `H(anchor)`

| Field | Value |
|---|---|
| **Phase** | PH30 — Panel sufficiency + attribution + reports |
| **Stage** | S5 — Loom + Assay (DDA & Bits) |
| **Crate** | `calyx-assay` |
| **Files** | `crates/calyx-assay/src/sufficiency.rs` (≤500) |
| **Depends on** | PH28 T01 (KSG MI), PH28 T02 (random projection), PH29 T03 (n_eff) |
| **Axioms** | A2, A8 |
| **PRD** | `dbprdplans/07 §4`, `06 §1` |

## Goal

Implement `panel_sufficiency(anchor) -> PanelSufficiency` — the
substrate-sufficiency test from *The Oracle and the Kernel*. Computes
`I(panel; anchor)` (the joint MI of the whole panel about the outcome) and
`H(anchor)` (the anchor's self-entropy). The ratio `I/H` tells whether the
panel can close the question. `I ≪ H` → the architecture cannot predict this
outcome regardless of model complexity — expose as a red flag with deficit.

## Build (checklist of concrete, code-level steps)

- [x] Define `PanelSufficiency`:
  ```rust
  pub struct PanelSufficiency {
      pub i_panel: MiEstimate,       // I(panel; anchor)
      pub h_anchor: EntropyEstimate, // H(anchor)
      pub deficit: f32,              // H(anchor) - I(panel; anchor)
      pub ratio: f32,                // I(panel; anchor) / H(anchor)  (0.0–1.0)
      pub verdict: Sufficient | Insufficient { deficit_bits: f32 },
      pub per_slot_attribution: Vec<SlotAttribution>,  // filled by T02
      pub anchor: AnchorKind,
      pub trust: Trusted | Provisional,
  }
  ```
- [x] Define `EntropyEstimate`: `{ bits: f32, n_samples: usize, ci_low: f32, ci_high: f32 }`
- [x] Implement `estimate_anchor_entropy(anchor_labels: &[u32], n_classes: usize) -> Result<EntropyEstimate, CalyxError>`:
  - compute frequency counts per class; `H = −Σ p_k · log2(p_k)` (with Laplace smoothing +0.5 per class)
  - bootstrap CI (200 resamples, seed=0)
  - if n < 50 → `CALYX_ASSAY_INSUFFICIENT_SAMPLES`
- [x] Implement `panel_mi(panel: &Panel, anchor_labels: &[u32], vault, forge, clock) -> Result<MiEstimate, CalyxError>`:
  - if N ≤ 5: concatenate all slot vectors and call `ksg_with_ci` on the concatenated panel vector after random projection
  - if N > 5: chain rule approximation `I(panel;Y) ≈ Σ I(slot_k;Y) − Σ pairwise_redundancy_nmi/2` (an approximation; document as such; for exact, promote to the full joint KSG which is expensive)
  - tag `trust: Trusted` iff anchor is grounded (A2)
- [x] Implement `panel_sufficiency(anchor, panel, vault, forge, clock) -> Result<PanelSufficiency, CalyxError>`:
  - calls `panel_mi` and `estimate_anchor_entropy`; computes `deficit = h_anchor.bits − i_panel.bits`; `ratio = i_panel.bits / h_anchor.bits`
  - `verdict: Insufficient` iff `deficit > 0.1 bits` (configurable; default 0.1)
  - `per_slot_attribution` left empty (filled by T02)

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: planted sufficient panel — 5 slots each with MI ≈ 0.2 bits (correlated with a binary anchor, n=300, seed=42) → `ratio ≥ 0.8`; `verdict: Sufficient`
- [x] unit: planted insufficient panel — 5 slots all uncorrelated with the anchor (MI ≈ 0.0 bits, n=300, seed=43) → `deficit ≈ H(anchor) ≈ 1.0 bit`; `verdict: Insufficient { deficit_bits ≈ 1.0 }`
- [x] unit: `estimate_anchor_entropy` for a balanced binary anchor (50% Pass, 50% Fail, n=200) → `H ≈ 1.0 bit ± 0.1`
- [x] proptest: `deficit = h_anchor.bits − i_panel.bits ≥ 0.0` always (DPI: panel MI cannot exceed anchor entropy)
- [x] edge: n=30 labeled samples → `CALYX_ASSAY_INSUFFICIENT_SAMPLES`; anchor with a single class (H=0) → `deficit=0.0, verdict: Sufficient` (trivially); panel with zero active slots → `i_panel=0.0, verdict: Insufficient`

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `panel_sufficiency` result persisted for a planted-insufficient panel (`I ≈ 0.46 bits`, `H ≈ 1.0 bit`)
- **Readback:**
  ```
  cargo test panel_insufficiency_planted -- --nocapture
  ```
  Output: `i_panel ≈ 0.46, h_anchor ≈ 1.0, deficit ≈ 0.54, verdict: Insufficient`.
- **Prove:** run on aiwonder; confirm the planted values are within the expected ranges. Also confirm the `trust: Trusted` tag only appears when the anchor is grounded.

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] FSV evidence (readback output / screenshot) attached to the PH30 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
