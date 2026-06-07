# PH38 · T01 — Conformal τ calibration per slot — ROC + quantile

| Field | Value |
|---|---|
| **Phase** | PH38 — τ Calibration (Conformal) + Novelty → New Region |
| **Stage** | S8 — Ward Gτ Guard |
| **Crate** | `calyx-ward` |
| **Files** | `crates/calyx-ward/src/calibrate.rs` (≤500) |
| **Depends on** | PH37 T01 (`GuardProfile`) · PH28 (grounded outcomes `AnchoredSet`) |
| **Axioms** | A2, A12 |
| **PRD** | `dbprdplans/09 §3` |

## Goal

Implement `calibrate()`: given an anchored set of known-good and known-bad
cosine scores per slot, use the conformal prediction quantile method to choose
`τ[slot_k]` that bounds the false-accept rate at the target `(1 − α)` confidence
level. Each slot gets its own `τ`. Identity slots are calibrated strict (lower
FAR target); stylistic slots loose. The result is a `GuardProfile` whose
`calibration` field is populated with full provenance. Default cold-start τ ≈
0.7 is used only when no calibration data exists; the calibrated value governs
(`09 §3`).

## Build (checklist of concrete, code-level steps)

- [ ] Define `CalibrationInput` struct:
      `slot: SlotId`, `good_scores: Vec<f32>` (cos of known-good outputs),
      `bad_scores: Vec<f32>` (cos of known-bad / injection outputs),
      `slot_kind: SlotKind` (`Identity | Stylistic | Content`),
      `target_far: f32` (e.g. 0.01 for identity, 0.05 for stylistic)
- [ ] Define `SlotKind` enum: `Identity | Stylistic | Content` — drives FAR
      target; identity slots use strict FAR ≤ 0.01, stylistic ≤ 0.05
- [ ] Implement `calibrate_slot(input: &CalibrationInput, alpha: f32,
      clock: &dyn Clock) -> Result<(f32, CalibrationMeta), WardError>`:
      - Require `input.bad_scores.len() >= 50` → else return
        `Err(WardError::InsufficientCalibrationData { n: len, min: 50 })`
        (maps to `CALYX_GUARD_PROVISIONAL`)
      - Conformal quantile: sort `bad_scores` ascending; `tau = quantile at
        (1 − target_far)` (i.e. the value below which `1 − target_far`
        fraction of bad scores fall — equivalently, only `target_far` fraction
        of bad scores exceed τ)
      - Compute achieved `far = fraction of bad_scores > tau`
      - Compute `frr = fraction of good_scores < tau`
      - `confidence = 1.0 - alpha`
      - `corpus_hash`: SHA-256 of sorted concatenated score bytes (stable,
        deterministic)
      - `estimator = "conformal_quantile_v1"`
      - Return `(tau, CalibrationMeta { corpus_hash, estimator, far, frr,
        confidence, ts: clock.now_micros() })`
- [ ] Implement `calibrate(profile_template: GuardProfile,
      inputs: Vec<CalibrationInput>, alpha: f32, clock: &dyn Clock)
      -> Result<GuardProfile, WardError>`:
      - Call `calibrate_slot` for each slot in `inputs`
      - Update `profile_template.tau` with calibrated values
      - Set `profile_template.calibration = Some(...)` using the first slot's
        meta (or a merged hash of all slots' corpus_hashes)
      - Return updated profile
- [ ] Cold-start constant `TAU_COLD_START: f32 = 0.7` — used only in
      `GuardProfile::tau_for()` fallback; never the output of `calibrate()`

## Tests (synthetic, deterministic — known input → known bytes/number)

- [ ] unit: 100 bad scores drawn from `Normal(0.4, 0.1)` (seed=42), 100 good
      scores from `Normal(0.85, 0.05)` (seed=42); `target_far=0.01`, `alpha=0.05`;
      assert returned τ in `[0.55, 0.75]`; assert `achieved_far ≤ 0.01`
- [ ] unit: same setup for identity slot (`target_far=0.01`) vs stylistic slot
      (`target_far=0.05`); assert identity τ ≥ stylistic τ (identity is stricter)
- [ ] proptest: for any bad_scores of length ≥ 50 with values in `[0.0, 1.0]`,
      achieved FAR of the returned τ ≤ target_far (conformal guarantee holds)
- [ ] edge: exactly 50 bad scores → `Ok` returned (boundary quorum)
- [ ] edge: 49 bad scores → `WardError::InsufficientCalibrationData { n: 49 }`
- [ ] edge: all bad scores = 0.99 → τ = 0.99 or 1.0; achieved_far = 0.0
- [ ] fail-closed: `target_far = 0.0` → τ set to maximum bad score; no division
      by zero

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** `CalibrationMeta` written to stdout via `serde_json::to_string_pretty`
  in test output
- **Readback:**
  `cargo test -p calyx-ward calibrate_slot -- --nocapture 2>&1 | grep -E "far|tau|estimator"`
- **Prove:** output shows `"estimator": "conformal_quantile_v1"`, `"far"` value
  ≤ 0.01, `"tau"` in the expected range; identity slot τ > stylistic slot τ;
  all tests `ok`

## Done when

- [ ] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [ ] file(s) ≤ 500 lines (line-count gate ✅)
- [ ] FSV evidence (readback output / screenshot) attached to the PH38 GitHub issue
- [ ] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
