# PH47 · T01 — Deficit localization (Assay attribution → deficit class)

| Field | Value |
|---|---|
| **Phase** | PH47 — Lens Proposal (Sufficiency Deficit) |
| **Stage** | S10 — Anneal + Intelligence Objective J |
| **Crate** | `calyx-anneal` |
| **Files** | `crates/calyx-anneal/src/propose/deficit_localize.rs` (≤500) |
| **Depends on** | — (first card; uses PH30 Assay attribution reports via trait) |
| **Axioms** | A7 |
| **PRD** | `dbprdplans/12 §5`, `dbprdplans/07 §4` |

## Goal

Implement `DeficitLocalizer`: given Assay's per-sensor attribution report
(`bits_per_anchor` per lens per outcome class), identify which outcome classes
have the largest sufficiency gap (`H(anchor_class) − I(panel; anchor_class)`)
and which input modalities / anchor types are under-covered. The localizer
outputs a `DeficitMap` that the candidate synthesizer (T02) uses to target the
most under-represented outcome class.

## Build (checklist of concrete, code-level steps)

- [ ] `struct DeficitMap { top_gaps: Vec<AnchorGap>, underrepresented_modalities: Vec<ModalityId>, total_bits_deficit: f64 }` where `AnchorGap { anchor_class: String, entropy_h: f64, mutual_info_i: f64, gap: f64 }`.
- [ ] `trait AssayAttribution { fn per_sensor_bits(&self, anchor: &AnchorId) -> Vec<(LensId, f64)>; fn panel_sufficiency(&self, anchor: &AnchorId) -> f64; fn entropy(&self, anchor: &AnchorId) -> f64; }` — bridged from `calyx-assay` (PH30).
- [ ] `fn localize(assay: &dyn AssayAttribution, anchor: &AnchorId, panel: &[LensId]) -> DeficitMap` — calls `panel_sufficiency` + `entropy`; for each lens calls `per_sensor_bits`; computes `gap = entropy − sufficiency`; sorts by gap descending; identifies modalities where no lens contributes > `0.10 bits`.
- [ ] `fn has_deficit(map: &DeficitMap, threshold: f64) -> bool` — returns `true` if `map.total_bits_deficit > threshold`; default threshold `0.5 bits` (configurable).
- [ ] `fn top_gap_description(map: &DeficitMap) -> String` — human-readable description for Ledger entry (e.g., "Anchor class 'outcome_positive' has gap 1.2 bits; no lens covers 'audio' modality").
- [ ] Clock-injected; no `SystemTime::now()`.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: `entropy=2.0 bits`, `panel_sufficiency=0.4 bits`, single lens at `0.4 bits` → `gap=1.6`, `total_bits_deficit=1.6`, `has_deficit(0.5)=true`.
- [ ] unit: `entropy=1.0 bits`, `panel_sufficiency=0.95 bits` → `gap=0.05`, `has_deficit(0.5)=false`.
- [ ] proptest: `gap = entropy − sufficiency` is always non-negative (MI ≤ H, guaranteed by the DPI/A8 ceiling in Assay).
- [ ] edge: empty panel (no lenses) → `DeficitMap` with `total_bits_deficit = entropy`; single lens → top gap correctly attributed to it; `entropy=0` → `gap=0`, no deficit.
- [ ] fail-closed: `AssayAttribution` returns `Err` → `CALYX_ASSAY_UNAVAILABLE`; `localize` returns the error, does not proceed with a zero-gap fake result.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `DeficitMap` computed from Assay's live attribution data.
- **Readback:** `calyx anneal deficit-map --anchor <anchor_id>` — prints `top_gaps`, `underrepresented_modalities`, `total_bits_deficit`, `has_deficit`.
- **Prove:** on a synthetic panel with known low sufficiency (`I=0.3, H=2.0`); `deficit-map` shows `gap=1.7`, `has_deficit=true`; on a well-covered panel (`I=1.9, H=2.0`), `has_deficit=false`.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH47 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
