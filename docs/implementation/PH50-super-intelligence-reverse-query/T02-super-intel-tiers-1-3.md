# PH50 · T02 — `super_intelligence`: tiers 1–3 (oracle_clean, panel_sufficient, kernel_exists)

| Field | Value |
|---|---|
| **Phase** | PH50 — Super-intelligence predicate + reverse_query |
| **Stage** | S11 — Oracle & AGI Layer |
| **Crate** | `calyx-oracle` |
| **Files** | `crates/calyx-oracle/src/super_intel.rs` (≤500) |
| **Depends on** | T01 (types), PH49 T02 (oracle_self_consistency), PH49 T03 (check_sufficiency), PH33 (kernel recall) |
| **Axioms** | A20, A2, A10 |
| **PRD** | `dbprdplans/21 §3`, `dbprdplans/21 §1` |

## Goal

Implement measurement of the first three tiers of the `super_intelligence` predicate:
- **Tier 1 — `oracle_clean`:** oracle self-consistency high (flakiness low, validity high); measured from `oracle_self_consistency(domain)`
- **Tier 2 — `panel_sufficient`:** `I(panel; oracle) ≥ τ_MI`; measured from `check_sufficiency(panel, domain)`
- **Tier 3 — `kernel_exists`:** Lodestar finds a grounded kernel; kernel-only recall ≥ 0.95 × full recall; measured from PH33 kernel recall API

Each tier is measured against **held-out** oracle outcomes (Goodhart-defended — test split never used during kernel construction or panel tuning). Short-circuits on first failure (conjunction).

## Build (checklist of concrete, code-level steps)

- [ ] `fn measure_tier_oracle_clean(vault: &Vault, domain: DomainId, clock: &dyn Clock) -> TierResult` — calls `oracle_self_consistency(domain)`; `passed = ceiling ≥ ORACLE_CLEAN_THRESHOLD` (default `0.7`); `measured_value = ceiling`; `cheapest_fix` if failing: "label more oracle instances to reduce flakiness" or "add validity-tracking anchor"
- [ ] `fn measure_tier_panel_sufficient(vault: &Vault, domain: DomainId, clock: &dyn Clock) -> TierResult` — calls `check_sufficiency`; `passed = I_panel_oracle ≥ H(outcome)` (same test as honesty gate); `measured_value = I_panel_oracle`; `cheapest_fix` if failing: per-sensor deficit's max-deficit sensor → "add outcome/execution-derived lens for <sensor_name>"
- [ ] `fn measure_tier_kernel_exists(vault: &Vault, domain: DomainId, clock: &dyn Clock) -> TierResult` — calls PH33 kernel recall API on the held-out split; `passed = kernel_recall ≥ 0.95 * full_recall`; `measured_value = kernel_recall / full_recall`; `cheapest_fix`: "ingest more anchor instances for domain"
- [ ] Each measurement is on **held-out** oracle outcomes: require a `HeldOutSplit` parameter that ensures no data leakage from training/ingest-time data
- [ ] `fn measure_tiers_1_to_3(vault: &Vault, domain: DomainId, held_out: &HeldOutSplit, clock: &dyn Clock) -> Vec<TierResult>` — runs all three; stops early (returns partial vec) if tier 1 or 2 fails (conjunction short-circuit is optional here; full measurement always preferable for diagnosis — make configurable via `ShortCircuit` flag)
- [ ] Thresholds as named constants: `ORACLE_CLEAN_THRESHOLD = 0.7`, `KERNEL_RECALL_RATIO = 0.95`

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: `oracle_self_consistency.ceiling = 0.8` → tier 1 passes; `ceiling = 0.5` → tier 1 fails with `cheapest_fix` non-empty
- [ ] unit: `I_panel_oracle = 1.05, H(outcome) = 1.0` → tier 2 passes; `I_panel_oracle = 0.46` → tier 2 fails with `cheapest_fix` naming the max-deficit lens
- [ ] unit: kernel recall = 0.96 × full recall → tier 3 passes; 0.93 × → tier 3 fails
- [ ] proptest: `TierResult.passed = (measured_value >= threshold)` always holds
- [ ] edge (≥3): no held-out data → tier 3 fails with `measured_value = 0.0` and `cheapest_fix = "label held-out instances"`; no oracle instances at all → tier 1 fails with `CALYX_ORACLE_NO_RECURRENCE` propagated; domain not found → all three tiers return `passed = false`
- [ ] fail-closed: any underlying query error → `TierResult { passed: false, cheapest_fix: <error description> }`; no panic, no silent pass

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** the `SuperIntelReport` JSON from `calyx readback super_intelligence <domain>`; `tiers[0..2]` fields
- **Readback:** `calyx readback super_intelligence --domain swe_bench_lite --tiers 1-3` prints tier 1–3 `TierResult` JSON; inspect `passed`, `measured_value`, `threshold`, `cheapest_fix`
- **Prove:** on a domain known to fail tier 2 (form-only panel), tier 2 `passed = false` and `cheapest_fix` names a specific lens; tier 1 on a clean oracle shows `passed = true` and `measured_value ≥ ORACLE_CLEAN_THRESHOLD`

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH50 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
