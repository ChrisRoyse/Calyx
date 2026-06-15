# PH50 · T03 — `super_intelligence`: tiers 4–6 (calibrated, goodhart_defended, mistake_closed)

| Field | Value |
|---|---|
| **Phase** | PH50 — Super-intelligence predicate + reverse_query |
| **Stage** | S11 — Oracle & AGI Layer |
| **Crate** | `calyx-oracle` |
| **Files** | `crates/calyx-oracle/src/super_intel.rs` (extend, ≤500) |
| **Depends on** | T02 (tiers 1–3 foundation), PH38 (τ calibration — tier 4), PH48 (J + Goodhart held-out — tier 5), PH45 (mistake-closure — tier 6) |
| **Axioms** | A20, A12, A14, A23 |
| **PRD** | `dbprdplans/21 §3`, `dbprdplans/17 §7.5` (Goodhart/hallucination) |

## Goal

Implement measurement of tiers 4–6 and the final `super_intelligence(domain)` entrypoint
that assembles all six `TierResult` values into a `SuperIntelReport`:
- **Tier 4 — `calibrated`:** predictor agrees with oracle at `τ_corr ≤ oracle_self_consistency`; measured from PH38 conformal calibration
- **Tier 5 — `goodhart_defended`:** Gτ + cross-lens anomaly resist gaming; measured from PH48 Goodhart held-out pass; `17 §7.5`
- **Tier 6 — `mistake_closed`:** online: wrong at most once, then healed; measured from PH45 mistake-closure replay

## Build (checklist of concrete, code-level steps)

- [ ] `fn measure_tier_calibrated(vault: &Vault, domain: DomainId, held_out: &HeldOutSplit, clock: &dyn Clock) -> TierResult` — call PH38 calibration score on held-out; `passed = calibration_error ≤ oracle_self_consistency.ceiling`; `cheapest_fix`: "run conformal calibration with more held-out instances"
- [ ] `fn measure_tier_goodhart_defended(vault: &Vault, domain: DomainId, held_out: &HeldOutSplit, clock: &dyn Clock) -> TierResult` — query PH48 J-objective Goodhart held-out result; `passed = goodhart_held_out_pass_rate ≥ GOODHART_THRESHOLD` (default `0.9`); `cheapest_fix`: "strengthen Gτ guard or add cross-lens anomaly detector" (`17 §7.5`)
- [ ] `fn measure_tier_mistake_closed(vault: &Vault, domain: DomainId, clock: &dyn Clock) -> TierResult` — query PH45 mistake-closure tracker; `passed = recurrence_of_same_mistake == 0` on replay buffer; `cheapest_fix`: "trigger online head update for the recurring mistake pattern"
- [ ] `pub fn super_intelligence(vault: &Vault, domain: DomainId, held_out: &HeldOutSplit, clock: &dyn Clock) -> Result<SuperIntelReport, OracleError>` — assembles all six tier results; builds `SuperIntelReport { domain, tiers, failing_tier, overall }`; writes a Ledger entry for the audit trail (A15)
- [ ] `failing_tier` = the **first** failing tier in the conjunction order: OracleClean → PanelSufficient → KernelExists → Calibrated → GoodhartDefended → MistakeClosed
- [ ] `cheapest_fix` on the report = `failing_tier_result.cheapest_fix` — the single most actionable fix
- [ ] Thresholds as named constants: `GOODHART_THRESHOLD = 0.9`, `CALIBRATION_CEILING_DELTA = 0.0` (calibration error ≤ ceiling exactly)

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: tier 4 — calibration error = 0.15, ceiling = 0.73 → passes (0.15 ≤ 0.73); calibration error = 0.8 → fails
- [ ] unit: tier 5 — Goodhart pass rate = 0.95 → passes; 0.85 → fails with `cheapest_fix` non-empty
- [ ] unit: tier 6 — 0 recurring mistakes → passes; 1 recurring mistake → fails
- [ ] unit: all 6 tiers pass → `super_intelligence` returns `Ok(report)` with `overall = true`, `failing_tier = None`
- [ ] unit: tier 3 fails → `failing_tier = Some(Tier::KernelExists)`; tiers 4–6 still measured (full diagnostic) but `failing_tier` reflects the first failure
- [ ] proptest: `report.overall ↔ report.tiers.iter().all(|t| t.passed)` always holds
- [ ] edge (≥3): held-out split empty → tier 4 + 5 fail with `cheapest_fix = "label held-out oracle instances"`; domain not found → `Err(OracleError::DomainNotFound)`; all tiers pass except tier 6 → `failing_tier = Some(Tier::MistakeClosed)`
- [ ] fail-closed: underlying phase query failure propagates as `OracleError`; no tier silently passes on error

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `SuperIntelReport` JSON from `calyx readback super_intelligence <domain>`; all 6 `TierResult` entries in `tiers` array; Ledger entry for the report
- **Readback:** `calyx readback super_intelligence --domain swe_bench_lite` prints full JSON; `jq '.failing_tier'` shows the failing tier name; `jq '.tiers[] | {tier: .tier, passed: .passed, measured_value: .measured_value}'` shows per-tier status
- **Prove:** `failing_tier` matches the first `false` in the tier list; `cheapest_fix` is non-empty on the failing tier; Ledger entry exists (read via `xxd`)

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH50 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
