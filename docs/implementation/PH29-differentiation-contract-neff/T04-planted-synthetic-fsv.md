# PH29 · T04 — Planted-synthetic FSV: redundant REJECTED, signal admitted, n_eff correct

> **Status: DONE in Stage 5 core.** The planted redundant/low-signal/n_eff
> cases are covered by the Stage 5 assay tests and the final JSON readback
> under `/home/croyse/calyx/data/fsv-stage5-loom-assay-20260608-final`.
> User-facing report commands are deferred to PH62.

| Field | Value |
|---|---|
| **Phase** | PH29 — Differentiation contract + n_eff |
| **Stage** | S5 — Loom + Assay (DDA & Bits) |
| **Crate** | `calyx-assay` |
| **Files** | `crates/calyx-assay/src/tests.rs` (≤500) |
| **Depends on** | T01, T02, T03 (all contract + n_eff implementations) |
| **Axioms** | A7, A8, A9, A16 |
| **PRD** | `dbprdplans/07 §3`, `07 §3c`, `15_STAGE5_LOOM_ASSAY.md` PH29 FSV gate |

## Goal

Write the three planted-synthetic FSV tests that close PH29 and are read as
byte-level proof on aiwonder: (1) a planted-redundant lens (corr > 0.6) is
REJECTED with `CALYX_ASSAY_REDUNDANT`; (2) a planted <0.05-bit lens is REJECTED
with `CALYX_ASSAY_LOW_SIGNAL`; (3) `n_eff` matches the known rank of a planted
panel. These tests read the stored decision rows from the assay CF to satisfy
FSV (not harness-level unit tests).

## Build (checklist of concrete, code-level steps)

- [x] Implement `test_planted_redundant_lens_rejected`:
  - create a test vault with a base panel containing one admitted lens `slot_a` with vectors drawn from `N(0,I)` (seed=1)
  - create candidate `slot_b` = `slot_a + N(0, 0.01·I)` (noise-perturbed copy; corr ≈ 0.99)
  - call `admit_lens(slot_b, anchor, panel)`
  - assert `AdmitResult::Reject { reason: Redundant }` with `max_corr > 0.6`
  - read the decision row from the assay CF: `calyx readback --cf assay --slot slot_b --decisions` must show `Rejected(Redundant)`
- [x] Implement `test_planted_low_signal_lens_rejected`:
  - candidate `slot_c` = random vectors uncorrelated with the anchor (MI ≈ 0.0 nats, seed=2)
  - call `admit_lens(slot_c, grounded_anchor, panel)` with n=200 labeled samples
  - assert `AdmitResult::Reject { reason: LowSignal }` with `bits < 0.05`
  - read the decision row: `Rejected(LowSignal)`
- [x] Implement `test_n_eff_planted_panel`:
  - create planted panel: 5 near-identical lenses (corr ≈ 0.92, seed=3) + 3 independent lenses (corr ≈ 0.0, seed=4); N=8
  - call `n_eff_panel(panel, vault, forge, clock)`
  - assert `n_eff ∈ [2.5, 4.0]` (known stable rank ≈ 3–4 for this construction)
  - read the abundance report: confirm n_eff shows `Computed { value ∈ [2.5, 4.0] }`
- [x] All three tests: seeded RNG only (`ChaCha8Rng`); no `thread_rng()`; no `Instant::now()`; use injected `FixedClock`
- [x] Add `#[test]` attribute and `#[cfg(test)]` module; tests run via `cargo test` not a custom harness

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] test_planted_redundant_lens_rejected → `Reject { reason: Redundant, max_corr > 0.6 }` (assertion in the test body)
- [x] test_planted_low_signal_lens_rejected → `Reject { reason: LowSignal, bits < 0.05 }` (assertion in the test body)
- [x] test_n_eff_planted_panel → `n_eff ∈ [2.5, 4.0]` (assertion in the test body)
- [x] regression: all three tests are deterministic across runs (run each test 3× on aiwonder; results identical)
- [x] edge: each test fails loudly if the vault setup step fails (no silent skip; `unwrap` with a clear message)

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** the assay CF decision rows and the `abundance_report` n_eff field on aiwonder
- **Readback:**
  ```
  cargo test test_planted_redundant_lens_rejected -- --nocapture
  cargo test test_planted_low_signal_lens_rejected -- --nocapture
  cargo test test_n_eff_planted_panel -- --nocapture
  calyx readback --cf assay --decisions --since 0
  cat /home/croyse/calyx/data/fsv-stage5-loom-assay-20260608-final/stage5-readback.json
  ```
- **Prove:**
  - CF rows show `Rejected(Redundant)` and `Rejected(LowSignal)` for the respective planted lenses
  - `abundance_report` shows `n_eff: Computed { value: f32 ∈ [2.5, 4.0] }` (not `[provisional]`)
  - all three `cargo test` runs succeed on aiwonder with no flakiness across 3 consecutive runs
  - Evidence (terminal screenshots + CF readback output) posted to the PH29 GitHub issue

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] FSV evidence (readback output / screenshot) attached to the PH29 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
