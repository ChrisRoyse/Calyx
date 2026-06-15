# PH50 · T05 — FSV: predicate reports failing tier; reverse recovers known cause

| Field | Value |
|---|---|
| **Phase** | PH50 — Super-intelligence predicate + reverse_query |
| **Stage** | S11 — Oracle & AGI Layer |
| **Crate** | `calyx-oracle` |
| **Files** | `crates/calyx-oracle/tests/super_intel_tests.rs` (≤500) |
| **Depends on** | T03 (`super_intelligence`), T04 (`reverse_query`) |
| **Axioms** | A20, A23, A2 |
| **PRD** | `dbprdplans/21 §3`, `dbprdplans/21 §5` |

## Goal

Prove the PH50 FSV exit gate on aiwonder:
1. `super_intelligence(domain)` reports `failing_tier` and `cheapest_fix` correctly on a real domain.
2. `reverse_query` on a known cause recovers it with `provisional = false`.
3. An ungrounded reverse query returns results labeled `provisional = true` — never fabricated as confident.

## Build (checklist of concrete, code-level steps)

- [ ] **FSV test 1 — failing tier report:** use SWE-bench Lite domain (form-only panel, known to fail tier 2 `PanelSufficient`); call `super_intelligence`; assert `report.failing_tier == Some(Tier::PanelSufficient)`; assert `cheapest_fix` contains the string "lens" (describing the missing outcome/execution sensor); write the full `SuperIntelReport` JSON to `/tmp/ph50_super_intel.json`
- [ ] **FSV test 2 — reverse_query recovers planted cause:** seed a synthetic vault with 20 co-occurrences of `code_change_X → test_failure_Y`; call `reverse_query(test_failure_Y, domain)`; assert the returned `Vec<Cause>` contains `action_or_event = "code_change_X"` with `provisional = false` as the first or tied-first result
- [ ] **FSV test 3 — ungrounded labeled provisional:** construct a vault with a structural association `A ↔ B` (same embedding space, no recurrence backing); call `reverse_query(B)`; assert all returned `Cause` entries have `provisional = true`
- [ ] **FSV test 4 — predicate passes when all tiers clear:** construct a synthetic domain with `oracle_self_consistency.ceiling = 0.85`, sufficient panel, valid kernel, good calibration, Goodhart pass, no recurring mistakes; `super_intelligence` returns `overall = true`, `failing_tier = None`
- [ ] Write FSV test 1 result to `/tmp/ph50_super_intel.json` in canonical JSON; write tests 2–3 results to `/tmp/ph50_reverse_query.json`
- [ ] Use `calyx-testkit` `MockClock` with seed `42`; all RNG deterministic

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] FSV test 1: `failing_tier = PanelSufficient`; `cheapest_fix` contains "lens"; `overall = false`
- [ ] FSV test 2: planted cause recovers; `provisional = false`; first in sorted order
- [ ] FSV test 3: all provisional; no `provisional = false` for ungrounded path
- [ ] FSV test 4: all tiers pass; `overall = true`
- [ ] edge: `reverse_query` with `answer` that matches zero constellations → `Err(OracleError::DomainNotFound)`
- [ ] fail-closed: tier 5 Goodhart query failure → `TierResult { passed: false, cheapest_fix: error description }`

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `/tmp/ph50_super_intel.json`, `/tmp/ph50_reverse_query.json`; stdout from `cargo test -p calyx-oracle -- super_intel --nocapture`
- **Readback:**
  ```
  cargo test -p calyx-oracle -- super_intel_tests --nocapture 2>&1 | tee /tmp/ph50_fsv.log
  cat /tmp/ph50_super_intel.json | jq '{failing_tier, overall, cheapest_fix: .tiers[] | select(.passed == false) | .cheapest_fix}'
  cat /tmp/ph50_reverse_query.json | jq '.causes[0] | {action_or_event, provisional}'
  grep "provisional.*false" /tmp/ph50_reverse_query.json   # planted cause: grounded
  grep "provisional.*true"  /tmp/ph50_reverse_query.json   # ungrounded: labeled
  ```
- **Prove:** `failing_tier = "PanelSufficient"` in JSON; planted cause appears with `provisional: false`; ungrounded path labeled `provisional: true`; `overall: true` for the all-pass synthetic domain

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH50 GitHub issue
- [ ] `/tmp/ph50_super_intel.json` and `/tmp/ph50_reverse_query.json` screenshots attached
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
