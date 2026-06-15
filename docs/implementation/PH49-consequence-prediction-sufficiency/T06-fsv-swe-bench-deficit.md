# PH49 · T06 — FSV: SWE-bench Lite ≈0.46-bit deficit + sufficiency-refusal

| Field | Value |
|---|---|
| **Phase** | PH49 — Consequence prediction + sufficiency gate |
| **Stage** | S11 — Oracle & AGI Layer |
| **Crate** | `calyx-oracle` |
| **Files** | `crates/calyx-oracle/tests/predict_tests.rs` (≤500) |
| **Depends on** | T04 (`oracle_predict`), T03 (honesty gate), T05 (butterfly tree), PH42 (recurrence), PH30 (sufficiency) |
| **Axioms** | A20, A2, A8 |
| **PRD** | `dbprdplans/21 §1` (the ME-JEPA honest negative result), `dbprdplans/21 §2` (honesty gate binding) |

## Goal

Prove the PH49 FSV exit gate on aiwonder:
1. On the SWE-bench Lite real deterministic-oracle domain, `oracle_predict` with a form-only
   panel measures `I(panel; oracle) ≈ 0.46 bits` and fires sufficiency-refusal.
2. On the same domain with a real code-execution panel (sufficient), prediction proceeds and
   confidence is capped at `oracle_self_consistency.ceiling`.
3. Confidence never exceeds the ceiling in any call — proven by structured test scanning
   all `Prediction` values returned across the test suite.

This card is the integration test and byte-readback harness for PH49. It does not contain
application logic; it is the FSV evidence factory.

## Build (checklist of concrete, code-level steps)

- [ ] `tests/predict_tests.rs`: integration test module; imports `calyx-oracle`, `calyx-assay`, `calyx-testkit`
- [ ] **FSV test 1 — deficit fires:** load the SWE-bench Lite domain (300 instances, deterministic Pass/Fail oracle) from testkit fixtures; construct a form-only panel (lenses that measure syntactic code structure: token embeddings, AST shape, identifier frequency — no execution outcome signal); call `oracle_predict` on a test instance; assert `result.is_err()` and `err.is_insufficient()`; assert `bound.I_panel_oracle` is in `[0.40, 0.55]` (≈0.46 ± 0.05 tolerance for estimator variance)
- [ ] **FSV test 2 — ceiling never exceeded:** build a synthetic vault with known `oracle_self_consistency.ceiling = 0.73`; call `oracle_predict` 50 times with varying recurrence strengths; assert `prediction.confidence ≤ 0.73` for every call; write the 50 confidence values to a deterministic log file (`/tmp/calyx_oracle_ceiling_check.jsonl`) for readback
- [ ] **FSV test 3 — per-sensor deficit readback:** on the form-only panel, assert `bound.per_sensor_deficit` has ≥1 entry; the sum of per-sensor gaps ≈ `H(outcome) - I_panel_oracle ± 0.1`; print the deficit vector to stdout in JSON (FSV evidence)
- [ ] **FSV test 4 — recurrence extrapolation:** synthetic vault with planted recurrence: action X occurred 15 times, 14 resulting in `Pass`; `oracle_predict(X)` returns `outcome = Pass` and `confidence > 0.5`; confidence ≤ ceiling
- [ ] Use `calyx-testkit` clock injection (seeded `MockClock`); all RNG seeded with fixed seeds (e.g., `42`); tests are bit-reproducible on aiwonder
- [ ] Each FSV test writes a brief structured log to stdout (JSON) so `calyx readback` can display the evidence

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] FSV test 1: form-only panel → `CALYX_ORACLE_INSUFFICIENT`; `I_panel_oracle ∈ [0.40, 0.55]`
- [ ] FSV test 2: all 50 predictions satisfy `confidence ≤ 0.73` ceiling — read from `/tmp/calyx_oracle_ceiling_check.jsonl`
- [ ] FSV test 3: per-sensor deficit sums approximately to `H(outcome) - I_panel_oracle`
- [ ] FSV test 4: planted recurrence 14/15 Pass → `outcome = Pass`, `confidence > 0.5`, `confidence ≤ ceiling`
- [ ] edge: single test instance with no prior recurrence → `CALYX_ORACLE_NO_RECURRENCE` (not a zero-confidence prediction)
- [ ] fail-closed: corrupted recurrence CF → `CALYX_ORACLE_NO_RECURRENCE`; honesty gate never silently bypassed

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `/tmp/calyx_oracle_ceiling_check.jsonl` (ceiling check log); stdout of `cargo test -p calyx-oracle -- fsv` (deficit + per-sensor dump); the `Prediction` JSON bytes
- **Readback:**
  ```
  cargo test -p calyx-oracle -- fsv --nocapture 2>&1 | tee /tmp/ph49_fsv.log
  cat /tmp/calyx_oracle_ceiling_check.jsonl | jq '.confidence <= .ceiling'  # must all be true
  grep I_panel_oracle /tmp/ph49_fsv.log                                      # must show ~0.46
  grep CALYX_ORACLE_INSUFFICIENT /tmp/ph49_fsv.log                           # must appear
  ```
- **Prove:** `I_panel_oracle ≈ 0.46` present in log; `CALYX_ORACLE_INSUFFICIENT` present; all 50 confidence values ≤ ceiling; `outcome = Pass` for 14/15 planted recurrence case

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH49 GitHub issue
- [ ] `/tmp/calyx_oracle_ceiling_check.jsonl` screenshot shows all confidence ≤ ceiling
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
