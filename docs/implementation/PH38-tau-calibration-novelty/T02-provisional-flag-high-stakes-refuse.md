# PH38 · T02 — `provisional` flag + `CALYX_GUARD_PROVISIONAL` high-stakes refuse

> STATUS: DONE / FSV-signed-off on aiwonder for #265; hardened by #649 and
> #650.
> Initial implementation `5c23db5ee9e0f1f95ed8f4c67011b49984770385`;
> #649 evidence roots:
> `/home/croyse/calyx/data/fsv-issue649-guard-provisional-20260610` and
> `/home/croyse/calyx/data/fsv-issue649-ledger-provenance-20260610`.

| Field | Value |
|---|---|
| **Phase** | PH38 — τ Calibration (Conformal) + Novelty → New Region |
| **Stage** | S8 — Ward Gτ Guard |
| **Crate** | `calyx-ward` |
| **Files** | `crates/calyx-ward/src/calibrate.rs` (≤500), `crates/calyx-ward/src/guard.rs` (≤500) |
| **Depends on** | T01 (this phase) · PH37 T01, T03 |
| **Axioms** | A12, A16 |
| **PRD** | `dbprdplans/09 §3`, `09 §7` |

## Goal

An uncalibrated `GuardProfile` (`calibration: None`) is tagged `provisional`.
When `guard()` is called against a `provisional` profile in a high-stakes
domain, it must fail closed with `CALYX_GUARD_PROVISIONAL` rather than running
with the cold-start τ ≈ 0.7. The domain's high-stakes flag is a field on the
call site, not on the profile — the caller decides stake level. This enforces
the constraint from `09 §3` and `09 §7`: "calibration MUST be against grounded
outcomes; an uncalibrated τ is tagged `provisional` and high-stakes domains
MUST refuse."

## Build (checklist of concrete, code-level steps)

- [x] Add `high_stakes: bool` parameter to `guard()` signature:
      `guard(profile: &GuardProfile, produced: &ProducedSlots,
      matched: &MatchedSlots, high_stakes: bool) -> Result<GuardVerdict, WardError>`
- [x] At the top of `guard()`, before any slot iteration:
      `if high_stakes && !profile.is_calibrated() { return Err(WardError::Provisional
      { guard_id: profile.guard_id }) }`
- [x] #649 hardening: after deriving required slots, high-stakes guard calls
      require each required slot to have both an explicit tau and
      `CalibrationMeta.per_slot` provenance. A profile-level calibration summary
      without slot provenance fails closed with `CALYX_GUARD_PROVISIONAL`.
- [x] #650 hardening: inert-profile validation runs before high-stakes
      provenance checks, so empty required-slot profiles and `KofN { k: 0 }`
      return `CALYX_GUARD_INERT_PROFILE` instead of a vacuous pass.
- [x] `WardError::Provisional` display format:
      `"CALYX_GUARD_PROVISIONAL: guard {guard_id} is uncalibrated; calibrate
      before high-stakes use -- run calibrate() with an anchored set >=50 examples"`
- [x] When `!high_stakes && !profile.is_calibrated()`: guard proceeds using
      cold-start τ = `DEFAULT_TAU` (0.7) for any slot absent from the tau
      map, and the `GuardVerdict` carries a `provisional: true` boolean flag
      so the caller can observe it was run on an uncalibrated profile
- [x] Add `provisional: bool` field to `GuardVerdict`:
      `true` when `profile.calibration.is_none()`; `false` otherwise
- [x] Add `guard_non_high_stakes` convenience alias that calls `guard(..,
      high_stakes: false)` — for use in non-critical embeddings

## Tests (synthetic, deterministic — known input → known bytes/number)

- [x] unit: uncalibrated profile + `high_stakes=true` → `WardError::Provisional`
      returned; not a panic; display contains `"CALYX_GUARD_PROVISIONAL"`
- [x] unit: uncalibrated profile + `high_stakes=false` → `Ok(GuardVerdict)` with
      `provisional: true`; cos evaluated against τ=0.7 (cold-start)
- [x] unit: calibrated profile + `high_stakes=true` → proceeds normally; verdict
      `provisional: false`
- [x] unit: calibrated profile + `high_stakes=false` → proceeds normally; verdict
      `provisional: false`
- [x] edge: profile with `calibration: Some(..)` but a required slot missing
      from the tau map → high-stakes fails closed with
      `CALYX_GUARD_PROVISIONAL`; non-high-stakes still uses cold-start 0.7 and
      returns `provisional: false`
- [x] edge: profile-level `calibration: Some(..)` with empty
      `CalibrationMeta.per_slot` → high-stakes fails closed with
      `CALYX_GUARD_PROVISIONAL` naming the required slot
- [x] fail-closed: `WardError::Provisional` formatted string contains the advice
      "calibrate before high-stakes use" and the guard_id UUID

## FSV (read the bytes on aiwonder — the truth gate)

- **SoT:** durable aiwonder evidence root containing provisional refusal JSON,
  non-high-stakes provisional verdict JSON, captured log, and a SHA-256
  manifest; #649 also reads physical Ledger calibration/verdict row bytes.
- **Readback:** run the manual FSV fixture with
  `CALYX_WARD_PROVISIONAL_FSV_DIR=$root`, then separately inspect the JSON/log
  artifacts with `xxd`, `sha256sum`, grep, and parsed JSON.
- **Prove:** durable readback contains `CALYX_GUARD_PROVISIONAL`; the
  uncalibrated+high_stakes case records `Err(Provisional { .. })`, while the
  uncalibrated+non_high_stakes case records the provisional verdict path.
  #649 additionally proves missing high-stakes required-slot tau and missing
  required-slot calibration provenance both return `CALYX_GUARD_PROVISIONAL`,
  while a calibrated high-stakes call appends Guard Ledger rows at seqs `[0,1]`
  and the refused profile-level-only call appends no row.

## Done when

- [x] `cargo check` + `clippy -D warnings` + `test` green on aiwonder
- [x] file(s) ≤ 500 lines (line-count gate ✅)
- [x] FSV evidence (readback output / screenshot) attached to the PH38 GitHub issue
- [x] no anti-pattern (DOCTRINE §9): no flatten / no `C(N,2)` past DPI / nothing
      "trusted" without grounding / no frozen-lens mutation / no harness-as-FSV
