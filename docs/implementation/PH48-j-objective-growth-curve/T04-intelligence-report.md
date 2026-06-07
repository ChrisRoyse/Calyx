# PH48 · T04 — Intelligence report (per-term breakdown, DPI headroom)

| Field | Value |
|---|---|
| **Phase** | PH48 — J Objective + Growth Curve + Intelligence Report |
| **Stage** | S10 — Anneal + Intelligence Objective J |
| **Crate** | `calyx-anneal` |
| **Files** | `crates/calyx-anneal/src/j/intelligence_report.rs` (≤500) |
| **Depends on** | T01 (JValue), T02 (GoodhartReport), T03 (IntelligenceGradient) |
| **Axioms** | A32, A8 |
| **PRD** | `dbprdplans/27 §8` |

## Goal

Implement `intelligence_report(vault) -> IntelligenceReport` — the public API
that returns a complete snapshot of vault intelligence state: `J` value, per-term
breakdown with weights, `DPI_headroom`, `provisional_excluded` count, top-5
gradient actions (`ΔJ/cost`), last Goodhart check result, and the
`next_best_action`. The report is fully auditable: every number sourced from a
grounded measurement; nothing asserted without evidence. The `calyx anneal
intelligence-report` CLI command calls this.

## Build (checklist of concrete, code-level steps)

- [ ] `struct IntelligenceReport { j: f64, terms: JTerms, weights: JWeights, dpi_ceiling: f64, dpi_headroom: f64, provisional_excluded: usize, gradient: Vec<(CandidateAction, f64)>, next_best_action: Option<CandidateAction>, goodhart_last: Option<GoodhartReport>, ts: LogicalTime }`.
- [ ] `fn intelligence_report(vault: &Vault, sources: &dyn JMetricSources, gradient: &IntelligenceGradient, goodhart_state: &GoodhartState, clock: &dyn Clock) -> IntelligenceReport` — assembles all components; calls `compute_j`, reads `gradient.queue`, reads last Goodhart result.
- [ ] `fn format_report(report: &IntelligenceReport) -> String` — human-readable output matching the format expected by `calyx anneal intelligence-report`; each term labeled, indented, with its contribution to `J`; gradient top-5 with `ΔJ/cost`; next action highlighted.
- [ ] `fn to_json(report: &IntelligenceReport) -> serde_json::Value` — machine-readable form for downstream consumers (PH48 growth curve, PH70 intelligence validation).
- [ ] Report persisted to `anneal_report` CF snapshot under a logical timestamp key; allows historical comparison.
- [ ] `fn report_diff(before: &IntelligenceReport, after: &IntelligenceReport) -> ReportDiff { delta_j, per_term_deltas, new_gradient_top }` — used by growth curve (T05) to track change.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: given known `JValue` + known gradient with 3 actions → `IntelligenceReport.gradient` has exactly 3 entries in descending `dj_per_cost` order.
- [ ] unit: `format_report` output for a known report contains the exact string `"J = <value>"` and `"DPI headroom: <value>"` (snapshot test with known input).
- [ ] unit: `to_json(report)["j"]` equals `report.j` (round-trip via serde).
- [ ] edge: `provisional_excluded=0` → report still shows `"provisional_excluded: 0"`; empty gradient queue → `next_best_action: None` printed; `goodhart_last=None` → `"Goodhart: no check yet"`.
- [ ] fail-closed: `sources.mutual_info_panel_anchor()` returns `Err` → `IntelligenceReport` has `j=NaN` explicitly marked as unavailable; not silently zero.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `anneal_report` CF snapshot + CLI output.
- **Readback:** `calyx anneal intelligence-report` on aiwonder — full report printed.
- **Prove:** run `intelligence-report` on a vault with known state; confirm `J` value, all 8 term labels present, `DPI_headroom`, `provisional_excluded`, and top gradient action are all printed; run again after one autotune promotion → `report_diff` shows a positive `delta_j`.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH48 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
