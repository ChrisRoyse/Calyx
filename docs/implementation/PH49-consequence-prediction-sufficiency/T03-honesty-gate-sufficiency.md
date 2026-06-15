# PH49 · T03 — Honesty gate: `check_sufficiency` + `CALYX_ORACLE_INSUFFICIENT`

| Field | Value |
|---|---|
| **Phase** | PH49 — Consequence prediction + sufficiency gate |
| **Stage** | S11 — Oracle & AGI Layer |
| **Crate** | `calyx-oracle` |
| **Files** | `crates/calyx-oracle/src/honesty_gate.rs` (≤500) |
| **Depends on** | T02 (self-consistency), PH30 (panel sufficiency + per-sensor attribution), PH28 (KSG MI) |
| **Axioms** | A20, A2, A8, A16 |
| **PRD** | `dbprdplans/21 §1`, `dbprdplans/21 §2`, `dbprdplans/21 §8` |

## Goal

Implement the **honesty gate**: `check_sufficiency(panel, domain)` measures
`I(panel; oracle)` and `H(outcome)` for the domain via the Assay API, constructs a
`SufficiencyBound`, and — if `I(panel; oracle) < H(outcome)` — returns
`Err(OracleError::Insufficient { bound })` with `sufficient: false` and a per-sensor
deficit vector. This is the ME-JEPA discipline as a runtime guarantee: the Oracle
**refuses to fake a confident prediction the panel cannot support** (`21 §2`). It does
not re-implement MI estimation; it delegates to `calyx-assay` (PH30).

## Build (checklist of concrete, code-level steps)

- [ ] `fn check_sufficiency(vault: &Vault, panel: &Panel, domain: DomainId) -> Result<SufficiencyBound, OracleError>`
- [ ] Call PH30 `panel_sufficiency(domain)` → `I(panel; oracle)` (bits) and `H(outcome)` (bits) via the Assay trait interface
- [ ] Call PH30 per-sensor attribution to get `per_sensor_deficit: Vec<(LensId, f32)>` — each lens's individual `I(lens; oracle)` minus its proportional share of `H(outcome)`
- [ ] Populate `SufficiencyBound { I_panel_oracle, dpi_ceiling, sufficient, per_sensor_deficit }` where `dpi_ceiling = I_panel_oracle` (data-processing inequality: no predictor reading the panel can exceed this; `21 §1`)
- [ ] If `I_panel_oracle < H(outcome)`: return `Err(OracleError::Insufficient { bound })` — **never** proceed to prediction; include all deficit fields in the error for diagnosis
- [ ] If sufficient: return `Ok(bound)` with `sufficient: true`; caller may proceed to prediction
- [ ] The `≈0.46-bit deficit` case (SWE-bench Lite form-only panel): `I_panel_oracle ≈ 0.46`, `H(outcome) ≈ 1.0` → `sufficient: false`; per-sensor deficit pinpoints each lens's gap (A2 localization)
- [ ] All calls use injected `Clock`; no `SystemTime::now()`; all RNG seeded

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: panel with `I_panel_oracle = 0.46`, `H(outcome) = 1.0` → `sufficient: false`; `CALYX_ORACLE_INSUFFICIENT` error returned; `per_sensor_deficit` non-empty
- [ ] unit: panel with `I_panel_oracle = 1.05`, `H(outcome) = 1.0` → `sufficient: true`; `bound.sufficient = true`; no error
- [ ] unit: `dpi_ceiling` equals `I_panel_oracle` in the returned bound (DPI invariant)
- [ ] proptest: if `I_panel_oracle < H(outcome)` then result is always `Err`; if `I_panel_oracle >= H(outcome)` then always `Ok`
- [ ] edge (≥3): panel with 0 lenses → `CALYX_ORACLE_INSUFFICIENT` (trivially insufficient); `H(outcome) = 0.0` (deterministic oracle) → always sufficient; `I_panel_oracle = H(outcome)` exactly → `sufficient: true` (boundary)
- [ ] fail-closed: Assay call failure → propagates `OracleError` with remediation; never silently treats failure as sufficient

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `crates/calyx-oracle/src/honesty_gate.rs`; the returned `SufficiencyBound` JSON from `oracle_predict` readback
- **Readback:** on aiwonder with SWE-bench Lite domain using a form-only panel, run `calyx readback oracle_predict --domain swe_bench_lite_form_only`; the printed JSON must show `"sufficient": false`, `"I_panel_oracle": <value near 0.46>`, and a non-empty `"per_sensor_deficit"` array
- **Prove:** the `I_panel_oracle` field in the readback is ≈0.46 (within ±0.05); `sufficient` is `false`; `CALYX_ORACLE_INSUFFICIENT` appears in the error log

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH49 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
