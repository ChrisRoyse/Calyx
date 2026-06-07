# PH48 · T02 — Goodhart defense (held-out + Gτ + cross-lens anomaly)

| Field | Value |
|---|---|
| **Phase** | PH48 — J Objective + Growth Curve + Intelligence Report |
| **Stage** | S10 — Anneal + Intelligence Objective J |
| **Crate** | `calyx-anneal` |
| **Files** | `crates/calyx-anneal/src/j/goodhart.rs` (≤500) |
| **Depends on** | T01 (`JValue` used before and after candidate change) |
| **Axioms** | A32, A2, A8 |
| **PRD** | `dbprdplans/27 §6` |

## Goal

Implement `GoodharChecker`: the three Goodhart defenses that prevent a `J`-
raising promotion from being a metric-gaming artifact: (1) held-out validation
— `J` must rise on a reserved grounded set, not just the training set; (2) `Gτ`
guard check — the change must not inflate `J` by staying in-region (using Ward's
`Gτ`); (3) cross-lens anomaly — the change must not be explained by a single
lens's sudden jump (suggesting a label-leakage or shortcut). A promotion that
fails any check is reverted and `P_goodhart` is incremented in the vault state.

## Build (checklist of concrete, code-level steps)

- [ ] `enum GoodhartViolation { HeldOutRegression { j_train_delta: f64, j_heldout_delta: f64 }, GtauViolation { in_region_frac: f64, threshold: f64 }, CrossLensAnomaly { anomalous_lens: LensId, delta_fraction: f64 } }`.
- [ ] `struct GoodhartReport { passed: bool, violations: Vec<GoodhartViolation>, p_goodhart_increment: f64 }`.
- [ ] `struct GoodhartChecker { held_out_set: HeldOutSet, ward: Arc<dyn WardGtau>, substract_threshold: f64 }` — `HeldOutSet` is a fixed, sealed sample of grounded anchors reserved at vault creation; never used for training.
- [ ] `fn check(before: &JValue, after: &JValue, vault: &Vault, change: &dyn AnnealAction) -> GoodhartReport`:
  - (a) **Held-out**: compute `j_train_delta = after.j − before.j`; compute `j_heldout = compute_j(vault, held_out_sources)`; if `j_heldout <= j_before_heldout + 0.01 × j_train_delta` → `HeldOutRegression` violation.
  - (b) **Gτ**: call `ward.in_region_fraction(held_out_set)` after change; if `in_region_frac < 0.95` → `GtauViolation` (the change pushed data out of the guarded region, suggesting shortcut learning).
  - (c) **Cross-lens anomaly**: for each lens in panel, compute per-lens `J` contribution delta; if any single lens accounts for `> 0.80` of `j_train_delta` → `CrossLensAnomaly`.
- [ ] If any violation: set `p_goodhart_increment = |j_train_delta| × violation_penalty_weight`; return `passed=false`; caller calls `substrate.rollback_explicit`.
- [ ] `p_goodhart_increment` is added to `vault.p_goodhart_state` and reflected in the next `compute_j` call.

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: `j_train_delta=1.0`, `j_heldout_delta=−0.5` → `HeldOutRegression` violation; `passed=false`.
- [ ] unit: single lens accounts for `85%` of `j_train_delta` → `CrossLensAnomaly`; `passed=false`.
- [ ] unit: all checks pass (`j_heldout_delta > 0`, `in_region_frac = 0.98`, no single-lens dominance) → `passed=true`, `p_goodhart_increment=0.0`.
- [ ] proptest: `passed=true` implies `len(violations) == 0`; `passed=false` implies `len(violations) > 0`.
- [ ] edge: `held_out_set` is empty → skip held-out check (treat as passed) + log a warning; `in_region_frac` unavailable from Ward → treat as `0.0` (fail-safe, conservative).

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `GoodhartReport` + Ledger `GoodhartFailed` / `GoodhartPassed` entries.
- **Readback:** `calyx readback ledger --kind Anneal --action GoodhartFailed --last 3`.
- **Prove:** inject a change that raises `J` on the training set but not the held-out set (add a highly correlated duplicate lens); `GoodhartChecker` fires `HeldOutRegression`; Ledger shows `GoodhartFailed`; promotion is reverted; `P_goodhart` increases in the next `intelligence-report`. Attach Ledger snippet to PH48 GitHub issue.

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH48 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
