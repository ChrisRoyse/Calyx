# PH48 · T01 — J composite computation (all terms + penalties + DPI cap)

| Field | Value |
|---|---|
| **Phase** | PH48 — J Objective + Growth Curve + Intelligence Report |
| **Stage** | S10 — Anneal + Intelligence Objective J |
| **Crate** | `calyx-anneal` |
| **Files** | `crates/calyx-anneal/src/j/j_composite.rs` (≤500) |
| **Depends on** | — (first card; bridges Assay/Lodestar/Oracle metrics via traits) |
| **Axioms** | A32, A2, A8 |
| **PRD** | `dbprdplans/27 §2` |

## Goal

Implement `compute_j(vault, sources) -> JValue` where `JValue` contains the
full composite `J` and a per-term breakdown. All eight positive terms are read
from the appropriate engine metrics; the DPI ceiling (A8) caps each information
term; the three penalties are computed and subtracted. Ungrounded/provisional
targets are excluded from the positive terms and counted in `P_ungrounded`.
This is the mechanical implementation of the `27 §2` formula — verbatim.

## Build (checklist of concrete, code-level steps)

- [ ] `struct JTerms { w1_info: f64, w2_n_eff: f64, w3_sufficiency: f64, w4_kernel_recall: f64, w5_oracle_accuracy: f64, w6_mistake_rate: f64, w7_compression: f64, w8_coverage: f64, p_redundant: f64, p_ungrounded: f64, p_goodhart: f64 }`.
- [ ] `struct JValue { j: f64, terms: JTerms, dpi_ceiling: f64, dpi_headroom: f64, provisional_excluded: usize, weights: JWeights }` where `j = Σ(positive terms) − Σ(penalties)` (all terms weighted by `JWeights`).
- [ ] `struct JWeights { w1..w8: f64 }` — defaults from `27 §2`; configurable via `set_objective_weights`; stored in vault config.
- [ ] `trait JMetricSources { fn mutual_info_panel_anchor(&self) -> f64; fn n_eff(&self) -> f64; fn panel_sufficiency(&self, domain: &str) -> f64; fn kernel_recall(&self) -> f64; fn oracle_accuracy(&self) -> f64; fn mistake_rate(&self) -> f64; fn compression_yield(&self) -> f64; fn coverage(&self) -> f64; fn dpi_ceiling(&self) -> f64; fn provisional_count(&self) -> usize; }` — one trait bridging all engine metrics; each engine crate provides an impl.
- [ ] DPI cap: for `w1_info` and `w3_sufficiency`, apply `term.min(dpi_ceiling)` before weighting; if term exceeds ceiling, record `dpi_headroom = ceiling - term` (negative = clipped).
- [ ] `P_ungrounded`: any `mutual_info_panel_anchor` measurement flagged `provisional` (ungrounded target) is zeroed from `w1_info` and counted in `provisional_excluded`; `P_ungrounded = sources.provisional_count() as f64 * UNIT_PENALTY`.
- [ ] `P_redundant`: computed from `n_eff < panel.len()` gap: `P_redundant = max(0.0, (panel.len() - n_eff) as f64) * REDUNDANCY_PENALTY`.
- [ ] `P_goodhart`: 0.0 at compute time; set by Goodhart checker (T02) post-promotion; stored in vault state.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: all terms at known values with default weights → `j` is exactly the weighted sum minus penalties (verify to 6 decimal places with fixed-point arithmetic).
- [ ] unit: `w1_info = 3.0` but `dpi_ceiling = 2.0` → `j` uses `2.0` for `w1` term (DPI-capped); `dpi_headroom = -1.0`.
- [ ] unit: `provisional_count = 5` → `P_ungrounded = 5 × UNIT_PENALTY`; `j` is reduced by that amount.
- [ ] proptest: for any `JMetricSources` impl where all terms are in `[0.0, 10.0]`, `compute_j` returns a finite `f64` (no NaN, no overflow).
- [ ] edge: all terms zero → `j = 0.0`; all weights zero → `j = -(P_redundant + P_ungrounded + P_goodhart)` (penalties still apply); `dpi_ceiling = 0.0` → all info terms capped to 0.0.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `JValue` returned by `compute_j` on a real or synthetic vault.
- **Readback:** `calyx anneal intelligence-report` — prints `j`, per-term values, `dpi_ceiling`, `dpi_headroom`, `provisional_excluded`.
- **Prove:** on a synthetic vault with known metric values; compute `j` manually with the formula; `intelligence-report` output matches to 4 decimal places; `dpi_ceiling` printed; `provisional_excluded` count printed.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH48 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
