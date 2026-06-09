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

## Status

DONE / FSV-signed-off on aiwonder for #264. Implemented in
`crates/calyx-ward/src/calibrate.rs`, exported from `calyx-ward`, and covered by
`crates/calyx-ward/tests/calibrate_unit.rs`. Final implementation commit:
`f95c817eff6f`. Evidence root:
`/home/croyse/calyx/data/fsv-issue264-ph38-t01-20260609-f95c817`.

Readback facts:
- `identity-style-comparison.json` shows
  `estimator="conformal_quantile_v1"`, `identity_far=0.009999999776482582`,
  `identity_tau=0.5940000414848328`, `style_tau=0.5820000767707825`, and
  `identity_tau_gt_style_tau=true`.
- `insufficient-error.json` shows `CALYX_GUARD_PROVISIONAL` for 49 bad scores.
- `all-high-bad-scores.json` shows tied high bad scores get `tau=0.9900000691413879`
  and `far=0.0`.
- `quantile-ties.json` proves the Ward boundary predicate is honored:
  `tau_above_tie_score=true` and `far=0.0` for bad scores equal to the quantile.
- `zero-target-far.json` shows `tau_above_max_bad=true` and `far=0.0`.
- `loose-identity-error.json` shows `CALYX_GUARD_PROVISIONAL` when an identity
  slot asks for a looser FAR than the slot-kind cap.

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

- [x] Define `CalibrationInput` struct:
      `slot: SlotId`, `good_scores: Vec<f32>` (cos of known-good outputs),
      `bad_scores: Vec<f32>` (cos of known-bad / injection outputs),
      `slot_kind: SlotKind` (`Identity | Stylistic | Content`),
      `target_far: f32` (e.g. 0.01 for identity, 0.05 for stylistic)
- [x] Define `SlotKind` enum: `Identity | Stylistic | Content` — drives FAR
      target; identity slots use strict FAR ≤ 0.01, stylistic ≤ 0.05,
      content ≤ 0.03. Callers may request stricter targets, not looser ones.
- [x] Implement `calibrate_slot(input: &CalibrationInput, alpha: f32,
      clock: &dyn Clock) -> Result<(f32, CalibrationMeta), WardError>`:
      - Require `input.bad_scores.len() >= 50` → else return
        `Err(WardError::InsufficientCalibrationData { n: len, min: 50 })`
        (maps to `CALYX_GUARD_PROVISIONAL`)
      - Conformal quantile: sort `bad_scores` ascending; `tau = quantile at
        (1 − target_far)`, advanced one f32 ULP when ties at the quantile would
        make Ward accept too many bad scores.
      - Compute achieved `far = fraction of bad_scores >= tau`, matching
        Ward's `cos >= tau` pass predicate.
      - Compute `frr = fraction of good_scores < tau`
      - `confidence = 1.0 - alpha`
      - `corpus_hash`: SHA-256 of sorted concatenated score bytes (stable,
        deterministic)
      - `estimator = "conformal_quantile_v1"`
      - Return `(tau, CalibrationMeta { corpus_hash, estimator, far, frr,
        confidence, ts: clock-derived microsecond timestamp })`
- [x] Implement `calibrate(profile_template: GuardProfile,
      inputs: Vec<CalibrationInput>, alpha: f32, clock: &dyn Clock)
      -> Result<GuardProfile, WardError>`:
      - Call `calibrate_slot` for each slot in `inputs`
      - Update `profile_template.tau` with calibrated values
      - Set `profile_template.calibration = Some(...)` using the first slot's
        meta (or a merged hash of all slots' corpus_hashes)
      - Return updated profile
- [x] Cold-start constant `TAU_COLD_START: f32 = 0.7` — used only in
      `GuardProfile::tau_for()` fallback; never the output of `calibrate()`

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: deterministic synthetic 100 bad scores around `0.30..0.597`, 100
      good scores around `0.80..0.899`; `target_far=0.01`, `alpha=0.05`;
      assert returned τ in `[0.55, 0.75]`; assert `achieved_far ≤ 0.01`
- [x] unit: same setup for identity slot (`target_far=0.01`) vs stylistic slot
      (`target_far=0.05`); assert identity τ > stylistic τ (identity is stricter)
- [x] proptest: for any bad_scores of length ≥ 50 with values in `[0.0, 1.0]`,
      achieved FAR of the returned τ ≤ target_far (conformal guarantee holds)
- [x] edge: exactly 50 bad scores → `Ok` returned (boundary quorum)
- [x] edge: 49 bad scores → `WardError::InsufficientCalibrationData { n: 49 }`
- [x] edge: all bad scores = 0.99 → τ is advanced above 0.99; achieved_far = 0.0
- [x] edge: ties at the quantile are advanced above the tied score, so calibration
      does not underreport bad scores that Ward would accept via `cos >= tau`.
- [x] fail-closed: `target_far = 0.0` → τ set above the maximum bad score; no
      division by zero
- [x] fail-closed: slot kind FAR caps reject loose identity calibration with
      `CALYX_GUARD_PROVISIONAL`.

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** durable aiwonder evidence root containing calibration JSON,
  identity/style tau comparison JSON, edge-case error JSON, and a SHA-256
  manifest.
- **Readback:** run the manual FSV fixture with
  `CALYX_WARD_CALIBRATE_FSV_DIR=$root`, then separately inspect the JSON files
  with `xxd`, `sha256sum`, and parsed JSON.
- **Prove:** durable JSON shows `"estimator": "conformal_quantile_v1"`,
  identity-slot FAR <= 0.01, `tau` in the expected range, identity-slot tau >
  stylistic-slot tau, and edge-case files for quorum failure, all-high bad
  scores, quantile ties, zero target FAR, and loose identity FAR.

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] FSV evidence (readback output / screenshot) attached to the PH38 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
