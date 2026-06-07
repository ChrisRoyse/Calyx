# PH50 · T01 — `SuperIntelReport` types + 6-tier enum

| Field | Value |
|---|---|
| **Phase** | PH50 — Super-intelligence predicate + reverse_query |
| **Stage** | S11 — Oracle & AGI Layer |
| **Crate** | `calyx-oracle` |
| **Files** | `crates/calyx-oracle/src/types.rs` (extend, ≤500), `crates/calyx-oracle/src/super_intel.rs` (stub, ≤500) |
| **Depends on** | PH49 T01 (existing oracle types) |
| **Axioms** | A20, A23 |
| **PRD** | `dbprdplans/21 §3`, `dbprdplans/21 §5` |

## Goal

Define the complete type tree for the super-intelligence predicate and the reverse-query
result: `Tier` enum (6 variants), `TierResult`, `SuperIntelReport`, `Cause`. These are the
data contracts for T02–T04 in this phase and downstream (PH51).

## Build (checklist of concrete, code-level steps)

- [ ] `enum Tier { OracleClean, PanelSufficient, KernelExists, Calibrated, GoodhartDefended, MistakeClosed }` — exactly the six conjuncts from `21 §3`: `oracle_clean ∧ panel_sufficient ∧ kernel_exists ∧ calibrated ∧ goodhart_defended ∧ mistake_closed`
- [ ] `struct TierResult { tier: Tier, passed: bool, measured_value: f32, threshold: f32, cheapest_fix: Option<String> }` — `cheapest_fix` is a human-readable action (e.g., "add outcome-execution lens", "label 200 more anchor instances")
- [ ] `struct SuperIntelReport { domain: DomainId, tiers: Vec<TierResult>, failing_tier: Option<Tier>, overall: bool }` — `overall = tiers.iter().all(|t| t.passed)`; `failing_tier` = first failing tier in predicate order (conjunction short-circuits)
- [ ] `struct Cause { action_or_event: String, domain: DomainId, confidence: f32, provisional: bool, provenance: LedgerRef }` — `provisional: true` when the back-edge lacks grounded recurrence
- [ ] Add `impl SuperIntelReport { fn failing_tier_report(&self) -> Option<&TierResult> }` — returns the `TierResult` for `failing_tier`
- [ ] All types `#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]`; `SuperIntelReport` implements `Display` summarizing tier pass/fail count and the failing tier name
- [ ] Extend `src/types.rs` (already exists from PH49); keep file ≤500 lines; split to `src/super_intel_types.rs` if needed

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: construct `SuperIntelReport` where tiers 1–3 pass, tier 4 fails → `overall = false`, `failing_tier = Some(Tier::Calibrated)`
- [ ] unit: all 6 tiers pass → `overall = true`, `failing_tier = None`
- [ ] unit: `Display` output contains the failing tier name when `overall = false`
- [ ] proptest: `overall = tiers.iter().all(|t| t.passed)` always holds; `failing_tier` is `None` iff `overall`
- [ ] edge (≥3): empty `tiers` vec → `overall = true` (vacuous); single-tier failing → `failing_tier = Some(that_tier)`; all fail → `failing_tier = Some(Tier::OracleClean)` (first in order)
- [ ] fail-closed: `SuperIntelReport` serde round-trip byte-identical

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `crates/calyx-oracle/src/types.rs` (or `super_intel_types.rs`); `grep -n "enum Tier"` shows 6 variants
- **Readback:** `cargo test -p calyx-oracle -- super_intel_types --nocapture` shows all assertions pass; `grep "Tier::" crates/calyx-oracle/src/types.rs | wc -l` shows 6 variants
- **Prove:** all 6 tier variants present; `SuperIntelReport::overall` logic correct on three known inputs; serde round-trip byte-identical

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH50 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
